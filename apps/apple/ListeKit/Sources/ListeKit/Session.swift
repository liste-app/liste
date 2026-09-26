// The app's connection to the core: one in-process host, one session.
//
// Everything the views show comes from here, and everything they change
// goes through here to the core as ops. No date parsing, merging, or
// storage happens in Swift; the session forwards text and ids and renders
// what the core returns. Refreshes are driven by the core's change
// notification, so a task added from the CLI or the MCP server appears
// without polling.

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
    case filter(SavedFilter)

    public var isSearch: Bool {
        if case .search = self { return true }
        return false
    }
}

/// A task with its subtasks, for display. Built from the core's flat rows;
/// nothing here decides anything.
public struct TaskNode: Identifiable, Hashable, Sendable {
    public let task: TaskItem
    public var children: [TaskNode]
    public let depth: Int
    public var id: String { task.id }

    public init(task: TaskItem, children: [TaskNode] = [], depth: Int = 0) {
        self.task = task
        self.children = children
        self.depth = depth
    }
}

@MainActor
@Observable
public final class Session {
    public private(set) var lists: [TaskList] = []
    public private(set) var tags: [Tag] = []
    public private(set) var tasks: [TaskItem] = []
    public private(set) var isLocked: Bool = false
    public private(set) var lastError: String?
    public private(set) var overdueCount: Int = 0
    /// The selection before a search began, restored on Escape.
    public private(set) var selectionBeforeSearch: Selection?
    public var selection: Selection = .today {
        didSet { refresh() }
    }
    /// How many days `upcoming` looks ahead.
    public var upcomingDays: UInt32 = 14
    /// How long the last refresh's core queries took, for the debug HUD.
    public private(set) var lastQueryMilliseconds: Double = 0
    /// Called after every refresh, for work that follows the store such as
    /// rescheduling reminders.
    public var onRefresh: (@MainActor () -> Void)?

    private let host: ListeHost
    private var forwarder: ChangeForwarder?
    private var pendingRefresh: Task<Void, Never>?

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

    /// Re-read lists, tags, counts, and the selected tasks from the core.
    public func refresh() {
        let started = Date()
        do {
            isLocked = host.isLocked()
            lists = try host.lists()
            tags = try host.tags()
            let now = Int64(Date().timeIntervalSince1970 * 1000)
            overdueCount = try host.query(query: Query(dueTo: startOfToday(), limit: 0)).count
            tasks = try fetch(selection, now: now)
            lastError = nil
        } catch {
            lastError = describe(error)
        }
        lastQueryMilliseconds = Date().timeIntervalSince(started) * 1000
        onRefresh?()
    }

    private func fetch(_ selection: Selection, now: Int64) throws -> [TaskItem] {
        switch selection {
        case .today: return try host.today()
        case .upcoming: return try host.upcoming(days: upcomingDays)
        case .anytime: return try host.query(query: Query(order: "manual"))
        case .completed: return try host.query(query: Query(completedOnly: true, order: "completed", limit: 500))
        case .inbox: return try host.tasksInList(listId: nil)
        case .list(let id): return try host.tasksInList(listId: id)
        case .tag(let id): return try host.query(query: Query(tagId: id))
        case .search(let text):
            return text.isEmpty ? [] : try host.search(query: text, limit: 200)
        case .filter(let f): return try host.query(query: f.query)
        }
    }

    /// The selected tasks as a tree: top-level tasks with their subtasks
    /// nested beneath them, in manual order.
    public var tree: [TaskNode] {
        Session.tree(of: tasks)
    }

    public static func tree(of tasks: [TaskItem]) -> [TaskNode] {
        let ids = Set(tasks.map(\.id))
        var byParent: [String?: [TaskItem]] = [:]
        for t in tasks {
            let parent = t.parentId.flatMap { ids.contains($0) ? $0 : nil }
            byParent[parent, default: []].append(t)
        }
        func build(_ parent: String?, depth: Int) -> [TaskNode] {
            (byParent[parent] ?? []).map { TaskNode(task: $0, children: build($0.id, depth: depth + 1), depth: depth) }
        }
        return build(nil, depth: 0)
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

    /// Move a task one step up or down among its siblings in the current
    /// list, by keyboard.
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
            lastError = nil
            return value
        } catch {
            lastError = describe(error)
            return nil
        }
    }

    private func describe(_ error: Error) -> String {
        (error as? ListeError).map { "\($0)" } ?? error.localizedDescription
    }

    private func startOfToday() -> Int64 {
        Int64(Calendar.current.startOfDay(for: Date()).timeIntervalSince1970 * 1000)
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
