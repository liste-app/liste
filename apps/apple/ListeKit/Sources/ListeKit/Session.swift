// The app's connection to the core: one in-process host, one session.
//
// Everything the views show comes from here, and everything they change
// goes through here to the core as ops. No date parsing, merging, or
// storage happens in Swift; the session forwards text and ids and renders
// what the core returns. Refreshes are driven by the core's change
// notification, so a task added from the CLI or the MCP server appears
// without polling.
//
// Listings are windows (Section 4: virtualize long lists). The session
// holds the count of rows in the selection and only the rows a view has
// asked for, plus a margin, so opening a 40,000-row list costs a count
// and one small fetch.

import Foundation
import ListeCore
import Observation

/// What the sidebar, search, or a saved filter can select.
public enum Selection: Hashable, Sendable {
    case today
    case upcoming
    case anytime
    case completed
    case inbox
    case list(id: String)
    case tag(id: String)
    case search(String)
    case filter(id: String)

    public var isSearch: Bool {
        if case .search = self { return true }
        return false
    }
}

@MainActor
@Observable
public final class Session {
    public private(set) var lists: [TaskList] = []
    public private(set) var tags: [Tag] = []
    public private(set) var filters: [SavedFilter] = []
    /// How many rows the selection has, whether or not they are loaded.
    public private(set) var count: Int = 0
    /// The loaded window: row `windowStart + i` is `tasks[i]`.
    public private(set) var tasks: [TaskItem] = []
    public private(set) var windowStart: Int = 0
    public private(set) var isLocked: Bool = false
    public private(set) var lastError: String?
    public private(set) var overdueCount: Int = 0
    /// The selection before a search began, restored on Escape.
    public private(set) var selectionBeforeSearch: Selection?
    public var selection: Selection = .today {
        didSet {
            wanted = 0..<0
            refresh(storeChanged: false)
        }
    }
    /// How many days `upcoming` looks ahead.
    public var upcomingDays: UInt32 = 14
    /// Rows fetched beyond what a view asked for, on each side.
    public var margin = 60
    /// Subtask groups the person collapsed, by parent id; a view preference
    /// that the query carries so the count and the rows agree.
    public var collapsed: Set<String> = Preferences.collapsedTasks {
        didSet {
            Preferences.collapsedTasks = collapsed
            refresh(storeChanged: false)
        }
    }
    /// How long the last refresh's core queries took, for measurements.
    /// Not observed: a number that changes on every refresh must not make
    /// views update.
    @ObservationIgnored public private(set) var lastQueryMilliseconds: Double = 0
    /// Counts refreshes, so a view with its own queries (the board) can
    /// re-run them after every change.
    public private(set) var generation = 0
    /// Called after a refresh that followed a change to the store (not one
    /// that followed a change of selection), for work that tracks the
    /// data itself, such as rescheduling reminders.
    public var onStoreChange: (@MainActor () -> Void)?
    /// How many rows the first window of a selection holds, before a view
    /// asks for more: enough for a screen, small enough to open at once.
    public var firstWindow = 40

    private let host: ListeHost
    private var forwarder: ChangeForwarder?
    private var pendingRefresh: Task<Void, Never>?
    /// The rows the view last asked for; empty means the top of the list.
    private var wanted: Range<Int> = 0..<0

    /// Start the host: take the store lock, open the store at the platform
    /// data directory, then listen on the Section 3 socket. Throws
    /// `ListeError.StoreHeld` when another Liste process owns the store.
    public init(dataDir: String? = nil, socketPath: String? = nil) throws {
        host = try ListeHost.start(dataDir: dataDir, socketPath: socketPath)
        let forwarder = ChangeForwarder { [weak self] in
            Task { @MainActor in self?.scheduleRefresh() }
        }
        self.forwarder = forwarder
        host.setChangeListener(listener: forwarder)
        refresh()
    }

    public var status: HostStatus? {
        try? host.status()
    }

    // MARK: Reading

    /// Coalesce a burst of change notifications (a sync pull, a fixture, a
    /// multi-op commit from another client) into one refresh shortly after
    /// the last one. The person's own actions refresh synchronously.
    public func scheduleRefresh(delay milliseconds: Int = 60) {
        pendingRefresh?.cancel()
        pendingRefresh = Task { @MainActor [weak self] in
            if milliseconds > 0 {
                try? await Task.sleep(for: .milliseconds(milliseconds))
            } else {
                await Task.yield()
            }
            guard !Task.isCancelled else { return }
            self?.refresh()
        }
    }

    /// Re-read lists, tags, filters, counts, and the wanted window of the
    /// selection from the core, after a change to the store.
    public func refresh() {
        refresh(storeChanged: true)
    }

    /// A change of selection re-reads only the count and the window; the
    /// lists, tags, filters, and the overdue count only change with the
    /// store.
    private func refresh(storeChanged: Bool) {
        let started = Date()
        do {
            // Observable properties are assigned only when their value
            // changed, so views depending on them are not asked to update
            // for a refresh that found nothing new.
            if storeChanged {
                assign(&isLocked, host.isLocked())
                assign(&lists, try host.lists())
                assign(&tags, try host.tags())
                assign(&filters, try host.filters())
                assign(&overdueCount, Int(try host.count(query: Query(dueToDay: 0))))
            }
            try loadWindow()
            assign(&lastError, nil)
        } catch {
            assign(&lastError, describe(error))
        }
        lastQueryMilliseconds = Date().timeIntervalSince(started) * 1000
        generation += 1
        if storeChanged {
            onStoreChange?()
        }
    }

    /// Make sure rows `range` are loaded, fetching them with the margin
    /// when the window does not cover them. Views call this for the rows
    /// they are about to show; a covered range costs nothing.
    public func ensureLoaded(_ range: Range<Int>) {
        let clipped = range.clamped(to: 0..<max(count, 0))
        wanted = clipped
        if clipped.isEmpty || (clipped.lowerBound >= windowStart && clipped.upperBound <= windowStart + tasks.count) {
            return
        }
        do {
            try loadWindow()
            assign(&lastError, nil)
        } catch {
            assign(&lastError, describe(error))
        }
    }

    /// The row at `index`, if it is in the loaded window.
    public func task(at index: Int) -> TaskItem? {
        let i = index - windowStart
        return tasks.indices.contains(i) ? tasks[i] : nil
    }

    /// The row index of a task, if it is in the loaded window.
    public func index(of id: String) -> Int? {
        tasks.firstIndex { $0.id == id }.map { $0 + windowStart }
    }

    /// A task by id: from the window when it is there, else from the core.
    public func find(_ id: String?) -> TaskItem? {
        guard let id else { return nil }
        return tasks.first { $0.id == id } ?? (try? host.task(id: id))
    }

    private func loadWindow() throws {
        if case .search(let text) = selection {
            assign(&tasks, text.isEmpty ? [] : try host.search(query: text, limit: 200))
            assign(&windowStart, 0)
            assign(&count, tasks.count)
            return
        }
        var spec = spec(for: selection)
        assign(&count, Int(try host.count(query: spec.query)))
        // The first window is exactly a screen; the margin comes with the
        // first scroll, so opening a list pays for what it shows.
        var range = wanted.isEmpty ? 0..<firstWindow : (wanted.lowerBound - margin)..<(wanted.upperBound + margin)
        range = range.clamped(to: 0..<max(count, 0))
        spec.offset = range.lowerBound
        spec.limit = range.count
        assign(&tasks, range.isEmpty ? [] : try host.query(query: spec.query))
        assign(&windowStart, range.lowerBound)
    }

    /// The core query a selection stands for. Day windows are relative
    /// days that the core resolves in its zone.
    public func spec(for selection: Selection) -> QuerySpec {
        var q = QuerySpec()
        q.collapsed = Array(collapsed)
        switch selection {
        case .today:
            q.dueToDay = 1
            q.order = "due"
        case .upcoming:
            q.dueFromDay = 1
            q.dueToDay = 1 + Int64(upcomingDays)
            q.order = "due"
        case .anytime:
            break
        case .completed:
            q.completedOnly = true
            q.order = "completed"
        case .inbox:
            q.inbox = true
        case .list(let id):
            q.listId = id
        case .tag(let id):
            q.tagId = id
        case .filter(let id):
            q.filterId = id
        case .search:
            break
        }
        return q
    }

    /// A count for any query; for board columns.
    public func count(_ query: Query) -> Int {
        Int((try? host.count(query: query)) ?? 0)
    }

    /// A window of any query; for board columns.
    public func fetch(_ spec: QuerySpec, offset: Int, limit: Int) -> [TaskItem] {
        var q = spec
        q.offset = max(offset, 0)
        q.limit = max(limit, 0)
        return (try? host.query(query: q.query)) ?? []
    }

    /// Every status name on an open task, `open` first.
    public func statuses() -> [String] {
        (try? host.statuses()) ?? ["open"]
    }

    /// Direct subtasks of a task, live ones first in manual order.
    public func subtasks(of task: TaskItem) -> [TaskItem] {
        (try? host.query(query: Query(parentId: task.id, includeCompleted: true))) ?? []
    }

    /// Parse a line without creating anything, for live highlighting.
    public func preview(_ text: String) -> CapturePreview? {
        try? host.preview(text: text, timeZone: nil)
    }

    public func task(id: String) throws -> TaskItem {
        try host.task(id: id)
    }

    /// Open tasks with a reminder set.
    public func query(hasReminder: Bool) throws -> [TaskItem] {
        try host.query(query: Query(hasReminder: hasReminder))
    }

    public func search(_ query: String, limit: UInt32 = 50) -> [TaskItem] {
        (try? host.search(query: query, limit: limit)) ?? []
    }

    /// Enter search mode, remembering where to return.
    public func beginSearch() {
        if !selection.isSearch {
            selectionBeforeSearch = selection
        }
        selection = .search("")
    }

    public func setSearch(_ text: String) {
        selection = .search(text)
    }

    /// Leave search mode, back to the previous selection.
    public func endSearch() {
        selection = selectionBeforeSearch ?? .today
        selectionBeforeSearch = nil
    }

    // MARK: Writing (every call is one or more core ops)

    /// Create a task from one line, as the person typed it.
    @discardableResult
    public func capture(_ text: String) throws -> Captured {
        defer { refresh() }
        return try host.capture(text: text, timeZone: nil)
    }

    /// A new task in the current context: the selected list, tag, or the
    /// inbox. Returns the created task.
    @discardableResult
    public func newTask(_ title: String = "New task") -> TaskItem? {
        var line = title
        switch selection {
        case .list(let id):
            if let list = lists.first(where: { $0.id == id }) { line += " /\(list.title)" }
        case .tag(let id):
            if let tag = tags.first(where: { $0.id == id }) { line += " #\(tag.name)" }
        case .today: line += " today"
        default: break
        }
        return perform { try host.capture(text: line, timeZone: nil).task }
    }

    public func setCompleted(_ task: TaskItem, _ completed: Bool) {
        perform { completed ? try host.complete(id: task.id) : try host.uncomplete(id: task.id) }
    }

    public func rename(_ task: TaskItem, to title: String) {
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, trimmed != task.title else { return }
        update(task, TaskPatch(title: trimmed))
    }

    public func setNotes(_ task: TaskItem, _ notes: String) {
        guard notes != task.notes else { return }
        update(task, TaskPatch(notes: notes))
    }

    /// Due date as natural-language text; empty clears it.
    public func setDue(_ task: TaskItem, _ text: String) {
        update(task, TaskPatch(due: text))
    }

    public func setReminder(_ task: TaskItem, _ text: String) {
        update(task, TaskPatch(reminder: text))
    }

    public func setPriority(_ task: TaskItem, _ priority: String) {
        guard priority != task.priority else { return }
        update(task, TaskPatch(priority: priority))
    }

    public func setStatus(_ task: TaskItem, _ status: String) {
        guard status != task.status else { return }
        update(task, TaskPatch(status: status))
    }

    /// Move to a list by name; empty moves to the inbox.
    public func setList(_ task: TaskItem, _ name: String) {
        update(task, TaskPatch(list: name))
    }

    public func setParent(_ task: TaskItem, _ parentId: String?) {
        update(task, TaskPatch(parent: parentId ?? ""))
    }

    public func addTag(_ task: TaskItem, _ name: String) {
        update(task, TaskPatch(addTags: [name]))
    }

    public func removeTag(_ task: TaskItem, _ name: String) {
        update(task, TaskPatch(removeTags: [name]))
    }

    public func update(_ task: TaskItem, _ patch: TaskPatch) {
        perform { try host.update(id: task.id, patch: patch) }
    }

    public func delete(_ task: TaskItem) {
        perform { try host.delete(id: task.id) }
    }

    /// Move `task` in manual order to sit after `after` and before `before`.
    public func reorder(_ task: TaskItem, after: TaskItem?, before: TaskItem?) {
        perform { try host.reorder(id: task.id, after: after?.id, before: before?.id) }
    }

    /// Move a task one step up or down among its siblings in the loaded
    /// window, by keyboard.
    public func move(_ task: TaskItem, up: Bool) {
        let siblings = tasks.filter { $0.parentId == task.parentId }
        guard let index = siblings.firstIndex(where: { $0.id == task.id }) else { return }
        if up {
            guard index > 0 else { return }
            let before = siblings[index - 1]
            let after = index >= 2 ? siblings[index - 2] : nil
            reorder(task, after: after, before: before)
        } else {
            guard index + 1 < siblings.count else { return }
            let after = siblings[index + 1]
            let before = index + 2 < siblings.count ? siblings[index + 2] : nil
            reorder(task, after: after, before: before)
        }
    }

    @discardableResult
    public func createList(_ title: String) -> TaskList? {
        perform { try host.createList(title: title) }
    }

    public func renameList(_ list: TaskList, to title: String) {
        perform { try host.updateList(id: list.id, title: title, after: nil, before: nil) }
    }

    public func reorderList(_ list: TaskList, after: TaskList?, before: TaskList?) {
        perform { try host.updateList(id: list.id, title: nil, after: after?.id, before: before?.id) }
    }

    public func deleteList(_ list: TaskList) {
        perform { try host.deleteList(id: list.id) }
    }

    /// Save a filter (Section 5); it syncs like a list.
    @discardableResult
    public func createFilter(_ name: String, _ criteria: FilterCriteria) -> SavedFilter? {
        perform { try host.createFilter(name: name, definition: criteria) }
    }

    public func renameFilter(_ filter: SavedFilter, to name: String) {
        perform { try host.updateFilter(id: filter.id, name: name, definition: nil, after: nil, before: nil) }
    }

    public func redefineFilter(_ filter: SavedFilter, _ criteria: FilterCriteria) {
        perform { try host.updateFilter(id: filter.id, name: nil, definition: criteria, after: nil, before: nil) }
    }

    public func reorderFilter(_ filter: SavedFilter, after: SavedFilter?, before: SavedFilter?) {
        perform { try host.updateFilter(id: filter.id, name: nil, definition: nil, after: after?.id, before: before?.id) }
    }

    public func deleteFilter(_ filter: SavedFilter) {
        perform { try host.deleteFilter(id: filter.id) }
    }

    public func undo() {
        perform { try host.undo() }
    }

    public func redo() {
        perform { try host.redo() }
    }

    /// Fill the store with the fixture; for benches and the acceptance driver.
    @discardableResult
    public func populateFixture(tasks: UInt32, seed: UInt64 = 42) -> UInt32 {
        perform { try host.populateFixture(tasks: tasks, seed: seed) } ?? 0
    }

    /// Fill the store with the fixture unless it already holds at least
    /// `tasks` tasks. Returns whether it did.
    @discardableResult
    public func populateFixtureIfSmall(_ tasks: UInt32) -> Bool {
        var all = QuerySpec()
        all.includeCompleted = true
        guard count(all.query) < Int(tasks) else { return false }
        populateFixture(tasks: tasks)
        return true
    }

    /// Stop the host: release the store lock and remove the socket.
    public func shutdown() {
        host.setChangeListener(listener: nil)
        host.shutdown()
    }

    @discardableResult
    private func perform<T>(_ body: () throws -> T) -> T? {
        defer { refresh() }
        do {
            let value = try body()
            assign(&lastError, nil)
            return value
        } catch {
            assign(&lastError, describe(error))
            return nil
        }
    }

    /// Store a value only when it differs, so observation stays quiet.
    private func assign<T: Equatable>(_ property: inout T, _ value: T) {
        if property != value {
            property = value
        }
    }

    private func describe(_ error: Error) -> String {
        (error as? ListeError).map { "\($0)" } ?? error.localizedDescription
    }
}

/// A query under construction. The core's `Query` record is immutable;
/// this is the same set of fields as variables, for views that derive one
/// query from another (a board column from its selection, a window from a
/// listing).
public struct QuerySpec: Hashable, Sendable {
    public var filterId: String?
    public var listId: String?
    public var inbox = false
    public var tagId: String?
    public var untagged = false
    public var parentId: String?
    public var priority: String?
    public var status: String?
    public var dueFromDay: Int64?
    public var dueToDay: Int64?
    public var hasReminder = false
    public var includeCompleted = false
    public var completedOnly = false
    public var collapsed: [String] = []
    public var order: String?
    public var offset = 0
    public var limit = 0

    public init() {}

    public var query: Query {
        Query(
            filterId: filterId, listId: listId, inbox: inbox, tagId: tagId, untagged: untagged,
            parentId: parentId, priority: priority, status: status, dueFromDay: dueFromDay,
            dueToDay: dueToDay, hasReminder: hasReminder, includeCompleted: includeCompleted,
            completedOnly: completedOnly, collapsed: collapsed, order: order,
            offset: UInt32(max(offset, 0)), limit: UInt32(max(limit, 0)))
    }
}

/// Receives the core's change callback on whatever thread made the change
/// and hops to the main actor.
final class ChangeForwarder: ChangeListener, @unchecked Sendable {
    private let handler: @Sendable () -> Void

    init(_ handler: @escaping @Sendable () -> Void) {
        self.handler = handler
    }

    func onChange() {
        handler()
    }
}

extension TaskItem: Identifiable {}
extension TaskList: Identifiable {}
extension Tag: Identifiable {}
extension SavedFilter: Identifiable {}
