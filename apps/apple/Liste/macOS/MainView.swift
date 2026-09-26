// The main window: a sidebar of smart lists and lists, and the tasks of the
// selected one. Scaffolding for the real UI; every value shown comes from
// the core through the session.

import ListeCore
import ListeKit
import SwiftUI

struct MainView: View {
    @Bindable var session: Session

    var body: some View {
        NavigationSplitView {
            List(selection: $session.selection) {
                Section {
                    Label("Today", systemImage: "sun.max").tag(Selection.today)
                    Label("Upcoming", systemImage: "calendar").tag(Selection.upcoming)
                    Label("Inbox", systemImage: "tray").tag(Selection.inbox)
                }
                Section("Lists") {
                    ForEach(session.lists, id: \.id) { list in
                        Label(list.title, systemImage: "list.bullet").tag(Selection.list(id: list.id))
                    }
                }
            }
            .navigationSplitViewColumnWidth(min: 180, ideal: 220)
        } detail: {
            TaskListView(session: session)
        }
        .navigationTitle(title)
    }

    private var title: String {
        switch session.selection {
        case .today: "Today"
        case .upcoming: "Upcoming"
        case .inbox: "Inbox"
        case .list(let id): session.lists.first { $0.id == id }?.title ?? "List"
        }
    }
}

struct TaskListView: View {
    var session: Session

    var body: some View {
        Group {
            if session.isLocked {
                ContentUnavailableView("Locked", systemImage: "lock", description: Text("Unlock Liste to see your tasks."))
            } else if session.tasks.isEmpty {
                ContentUnavailableView("Nothing here", systemImage: "checkmark.circle", description: Text("Press Option-Space to capture a task."))
            } else {
                List(session.tasks, id: \.id) { task in
                    TaskRow(task: task) { done in
                        session.setCompleted(task, done)
                    }
                }
            }
        }
        .overlay(alignment: .bottom) {
            if let error = session.lastError {
                Text(error).font(.callout).foregroundStyle(.red).padding(8)
            }
        }
    }
}

struct TaskRow: View {
    let task: TaskItem
    var onToggle: @MainActor @Sendable (Bool) -> Void

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Toggle(isOn: Binding(get: { task.completedAt != nil }, set: onToggle)) {
                EmptyView()
            }
            .toggleStyle(.checkbox)
            .labelsHidden()
            .accessibilityLabel("Complete \(task.title)")
            VStack(alignment: .leading, spacing: 2) {
                Text(task.title.isEmpty ? "Untitled" : task.title)
                    .strikethrough(task.completedAt != nil)
                HStack(spacing: 8) {
                    if let due = task.due {
                        Label(due, systemImage: "calendar")
                    }
                    if let list = task.listTitle {
                        Text("/\(list)")
                    }
                    ForEach(task.tags, id: \.self) { tag in
                        Text("#\(tag)")
                    }
                    if task.priority != "none" {
                        Text("!\(task.priority)")
                    }
                    if task.recurrence != nil {
                        Image(systemName: "repeat")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(.vertical, 2)
    }
}
