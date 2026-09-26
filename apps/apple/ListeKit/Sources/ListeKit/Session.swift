// The app's connection to the core: one in-process host, one session.
//
// Everything the views show comes from here, and everything they change
// goes through here to the core. No date parsing, merging, or storage
// happens in Swift; the session forwards text and ids and renders what the
// core returns. Refreshes are driven by the core's change notification, so
// a task added from the CLI or the MCP server appears without polling.

import Foundation
import ListeCore
import Observation

/// What the sidebar can select.
public enum Selection: Hashable, Sendable {
    case today
    case upcoming
    case inbox
    case list(id: String)
}

@MainActor
@Observable
public final class Session {
    public private(set) var lists: [TaskList] = []
    public private(set) var tasks: [TaskItem] = []
    public private(set) var isLocked: Bool = false
    public private(set) var lastError: String?
    public var selection: Selection = .today {
        didSet { refresh() }
    }
    /// How many days `upcoming` looks ahead.
    public var upcomingDays: UInt32 = 7

    private let host: ListeHost
    private var forwarder: ChangeForwarder?

    /// Start the host: take the store lock, open the store at the platform
    /// data directory, then listen on the Section 3 socket. Throws
    /// `ListeError.StoreHeld` when another Liste process owns the store.
    public init(dataDir: String? = nil, socketPath: String? = nil) throws {
        host = try ListeHost.start(dataDir: dataDir, socketPath: socketPath)
        let forwarder = ChangeForwarder { [weak self] in
            Task { @MainActor in self?.refresh() }
        }
        self.forwarder = forwarder
        host.setChangeListener(listener: forwarder)
        refresh()
    }

    public var status: HostStatus? {
        try? host.status()
    }

    /// Re-read lists and the selected tasks from the core.
    public func refresh() {
        do {
            isLocked = host.isLocked()
            lists = try host.lists()
            tasks = switch selection {
            case .today: try host.today()
            case .upcoming: try host.upcoming(days: upcomingDays)
            case .inbox: try host.tasksInList(listId: nil)
            case .list(let id): try host.tasksInList(listId: id)
            }
            lastError = nil
        } catch {
            lastError = describe(error)
        }
    }

    /// Parse a line without creating anything, for live highlighting.
    public func preview(_ text: String) -> CapturePreview? {
        try? host.preview(text: text, timeZone: nil)
    }

    /// Create a task from one line, as the person typed it.
    @discardableResult
    public func capture(_ text: String) throws -> Captured {
        try host.capture(text: text, timeZone: nil)
    }

    public func setCompleted(_ task: TaskItem, _ completed: Bool) {
        perform {
            if completed {
                _ = try host.complete(id: task.id)
            } else {
                _ = try host.uncomplete(id: task.id)
            }
        }
    }

    public func undo() {
        perform { _ = try host.undo() }
    }

    public func redo() {
        perform { _ = try host.redo() }
    }

    public func task(id: String) throws -> TaskItem {
        try host.task(id: id)
    }

    public func search(_ query: String, limit: UInt32 = 50) -> [TaskItem] {
        (try? host.search(query: query, limit: limit)) ?? []
    }

    /// Stop the host: release the store lock and remove the socket.
    public func shutdown() {
        host.setChangeListener(listener: nil)
        host.shutdown()
    }

    private func perform(_ body: () throws -> Void) {
        do {
            try body()
            lastError = nil
        } catch {
            lastError = describe(error)
        }
        refresh()
    }

    private func describe(_ error: Error) -> String {
        (error as? ListeError).map { "\($0)" } ?? error.localizedDescription
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
