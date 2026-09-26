// The list view: the empty states around the task table.

import ListeCore
import ListeKit
import SwiftUI

struct TaskListView: View {
    @Bindable var session: Session
    @Bindable var ui: UIState

    var body: some View {
        // The table stays mounted across selections, so switching views
        // reloads it rather than building it; the empty states sit on top.
        TaskTable(session: session, ui: ui, showsDayColumn: session.selection == .upcoming)
            .overlay {
                if session.isLocked {
                    ContentUnavailableView("Locked", systemImage: "lock", description: Text("Unlock Liste to see your tasks."))
                        .background(Tokens.Colors.background)
                } else if session.count == 0 {
                    empty.background(Tokens.Colors.background)
                }
            }
        .onChange(of: ui.editRequest) { _, _ in
            if let id = ui.selectedTaskId { ui.editingTaskId = id }
        }
        .overlay(alignment: .bottom) {
            if let error = session.lastError {
                Text(error)
                    .font(Tokens.Typography.footnote)
                    .foregroundStyle(Tokens.Colors.overdue)
                    .padding(Tokens.Space.sm)
            }
        }
    }

    private var empty: some View {
        let (title, hint): (String, String) = switch session.selection {
        case .search(let q) where !q.isEmpty: ("No matches", "Nothing in titles or notes matches \u{201C}\(q)\u{201D}.")
        case .search: ("Search", "Type to search titles and notes.")
        case .completed: ("Nothing completed yet", "Completed tasks appear here.")
        default: ("Nothing here", "Press \u{2318}N to add a task, or \u{2325}Space from anywhere.")
        }
        return ContentUnavailableView(title, systemImage: "checkmark.circle", description: Text(hint))
    }
}
