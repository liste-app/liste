// Saved filters (Section 5): a person's own smart lists. The filter itself
// is evaluated by the core; only the definition lives here, as a view
// preference in UserDefaults.

import Foundation
import ListeCore

public struct SavedFilter: Codable, Hashable, Identifiable, Sendable {
    public var id: UUID
    public var name: String
    public var listId: String?
    public var tagId: String?
    public var priority: String?
    public var status: String?
    /// Due within the next `dueWithinDays` days (0: no due filter; -1: overdue only).
    public var dueWithinDays: Int
    public var includeCompleted: Bool

    public init(
        id: UUID = UUID(), name: String, listId: String? = nil, tagId: String? = nil,
        priority: String? = nil, status: String? = nil, dueWithinDays: Int = 0, includeCompleted: Bool = false
    ) {
        self.id = id
        self.name = name
        self.listId = listId
        self.tagId = tagId
        self.priority = priority
        self.status = status
        self.dueWithinDays = dueWithinDays
        self.includeCompleted = includeCompleted
    }

    /// The core query this filter stands for, evaluated now.
    public var query: Query {
        let calendar = Calendar.current
        let start = Int64(calendar.startOfDay(for: Date()).timeIntervalSince1970 * 1000)
        var dueFrom: Int64?
        var dueTo: Int64?
        if dueWithinDays > 0 {
            dueFrom = start
            dueTo = Int64((calendar.date(byAdding: .day, value: dueWithinDays, to: calendar.startOfDay(for: Date())) ?? Date()).timeIntervalSince1970 * 1000)
        } else if dueWithinDays < 0 {
            dueTo = start
        }
        return Query(
            listId: listId, tagId: tagId, priority: priority, status: status,
            dueFrom: dueFrom, dueTo: dueTo, includeCompleted: includeCompleted, order: "due")
    }
}

/// View preferences: never data, never anything the core owns.
@MainActor
public enum Preferences {
    private static let defaults = UserDefaults.standard

    public static var vimKeys: Bool {
        get { defaults.bool(forKey: "vimKeys") }
        set { defaults.set(newValue, forKey: "vimKeys") }
    }

    /// "system", "light", or "dark".
    public static var theme: String {
        get { defaults.string(forKey: "theme") ?? "system" }
        set { defaults.set(newValue, forKey: "theme") }
    }

    public static var defaultList: String {
        get { defaults.string(forKey: "defaultList") ?? "" }
        set { defaults.set(newValue, forKey: "defaultList") }
    }

    public static var savedFilters: [SavedFilter] {
        get {
            guard let data = defaults.data(forKey: "savedFilters") else { return [] }
            return (try? JSONDecoder().decode([SavedFilter].self, from: data)) ?? []
        }
        set { defaults.set(try? JSONEncoder().encode(newValue), forKey: "savedFilters") }
    }

    /// Kanban grouping: "status", "priority", "list", or "tag".
    public static var kanbanGroup: String {
        get { defaults.string(forKey: "kanbanGroup") ?? "status" }
        set { defaults.set(newValue, forKey: "kanbanGroup") }
    }

    public static func kanbanColumnOrder(for group: String) -> [String] {
        defaults.stringArray(forKey: "kanbanOrder.\(group)") ?? []
    }

    public static func setKanbanColumnOrder(_ order: [String], for group: String) {
        defaults.set(order, forKey: "kanbanOrder.\(group)")
    }

    public static func kanbanCollapsed(for group: String) -> Set<String> {
        Set(defaults.stringArray(forKey: "kanbanCollapsed.\(group)") ?? [])
    }

    public static func setKanbanCollapsed(_ set: Set<String>, for group: String) {
        defaults.set(Array(set).sorted(), forKey: "kanbanCollapsed.\(group)")
    }

    /// Subtask groups the person collapsed, by parent task id.
    public static var collapsedTasks: Set<String> {
        get { Set(defaults.stringArray(forKey: "collapsedTasks") ?? []) }
        set { defaults.set(Array(newValue).sorted(), forKey: "collapsedTasks") }
    }

    /// The quick-capture hotkey as a Carbon key code and modifier mask.
    public static var hotKey: (keyCode: UInt32, modifiers: UInt32) {
        get {
            let code = defaults.object(forKey: "hotKeyCode") as? UInt32 ?? 49  // space
            let mods = defaults.object(forKey: "hotKeyModifiers") as? UInt32 ?? 2048  // option
            return (code, mods)
        }
        set {
            defaults.set(newValue.keyCode, forKey: "hotKeyCode")
            defaults.set(newValue.modifiers, forKey: "hotKeyModifiers")
        }
    }
}
