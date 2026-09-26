// View preferences: how things look and what is open, never data. Saved
// filters are data and live in the core (Section 5); only the view state
// around them is here.

import Foundation

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

    /// The main window's view, "list" or "board", kept across launches.
    public static var viewMode: String {
        get { defaults.string(forKey: "viewMode") ?? "list" }
        set { defaults.set(newValue, forKey: "viewMode") }
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
