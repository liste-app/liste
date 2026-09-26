// The list view: rows with a checkbox, inline title editing, drag and
// keyboard reorder, collapsible subtasks, and day sections for Upcoming.
// SwiftUI's List on macOS is table-backed, so rows are virtualized.

import ListeCore
import ListeKit
import SwiftUI

/// One visible row: a task at a depth, flattened from the tree so the
/// table can virtualize it.
struct Row: Identifiable, Hashable {
    let task: TaskItem
    let depth: Int
    let hasChildren: Bool
    var id: String { task.id }
}

struct TaskListView: View {
    @Bindable var session: Session
    @Bindable var ui: UIState
    @State private var collapsed = Preferences.collapsedTasks
    @FocusState private var listFocused: Bool

    var body: some View {
        Group {
            if session.isLocked {
                ContentUnavailableView("Locked", systemImage: "lock", description: Text("Unlock Liste to see your tasks."))
            } else if session.tasks.isEmpty {
                empty
            } else if case .upcoming = session.selection {
                sectioned
            } else {
                flat
            }
        }
        .focused($listFocused)
        .onKeyPress(.space) { toggleSelected(); return .handled }
        .onKeyPress(.return) { beginEdit(); return .handled }
        .onKeyPress(.delete) { deleteSelected(); return .handled }
        .onKeyPress(.upArrow, phases: .down) { press in
            guard press.modifiers.contains(.option) else { return .ignored }
            moveSelected(up: true)
            return .handled
        }
        .onKeyPress(.downArrow, phases: .down) { press in
            guard press.modifiers.contains(.option) else { return .ignored }
            moveSelected(up: false)
            return .handled
        }
        .onKeyPress(characters: .init(charactersIn: "jkx")) { press in
            guard Preferences.vimKeys, press.modifiers.isEmpty else { return .ignored }
            switch press.characters {
            case "j": step(1)
            case "k": step(-1)
            default: toggleSelected()
            }
            return .handled
        }
        .onChange(of: ui.editRequest) { _, _ in beginEdit() }
        .onChange(of: session.tasks.map(\.id)) { _, ids in
            // Off the table's own update pass, which must not re-enter.
            if let selected = ui.selectedTaskId, !ids.contains(selected) {
                Task { @MainActor in ui.selectedTaskId = nil }
            }
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

    private var rows: [Row] {
        flatten(session.tree)
    }

    private func flatten(_ nodes: [TaskNode]) -> [Row] {
        var out: [Row] = []
        for node in nodes {
            out.append(Row(task: node.task, depth: node.depth, hasChildren: !node.children.isEmpty))
            if !node.children.isEmpty, !collapsed.contains(node.id) {
                out.append(contentsOf: flatten(node.children))
            }
        }
        return out
    }

    private var flat: some View {
        List(selection: $ui.selectedTaskId) {
            ForEach(rows) { row in
                rowView(row)
            }
            .onMove(perform: moveRows)
        }
        .listStyle(.inset)
    }

    private var sectioned: some View {
        List(selection: $ui.selectedTaskId) {
            ForEach(DueDisplay.byDay(session.tasks), id: \.heading) { group in
                Section(group.heading) {
                    ForEach(flatten(Session.tree(of: group.tasks))) { row in
                        rowView(row)
                    }
                }
            }
        }
        .listStyle(.inset)
    }

    private func rowView(_ row: Row) -> some View {
        TaskRowView(
            row: row,
            isEditing: ui.editingTaskId == row.task.id,
            isCollapsed: collapsed.contains(row.task.id),
            onToggle: { done in session.setCompleted(row.task, done) },
            onRename: { title in
                session.rename(row.task, to: title)
                ui.editingTaskId = nil
            },
            onCancelEdit: { ui.editingTaskId = nil },
            onCollapse: {
                if collapsed.contains(row.task.id) { collapsed.remove(row.task.id) } else { collapsed.insert(row.task.id) }
                Preferences.collapsedTasks = collapsed
            }
        )
        .tag(row.task.id)
        .listRowSeparator(.hidden)
        .contextMenu {
            Button(row.task.completedAt == nil ? "Complete" : "Mark Incomplete") {
                session.setCompleted(row.task, row.task.completedAt == nil)
            }
            Button("Rename") { ui.selectedTaskId = row.task.id; beginEdit() }
            Menu("Priority") {
                ForEach(["high", "medium", "low", "none"], id: \.self) { p in
                    Button(p.capitalized) { session.setPriority(row.task, p) }
                }
            }
            Menu("Move to List") {
                Button("Inbox") { session.setList(row.task, "") }
                ForEach(session.lists, id: \.id) { list in
                    Button(list.title) { session.setList(row.task, list.title) }
                }
            }
            Divider()
            Button("Delete", role: .destructive) { session.delete(row.task) }
        }
    }

    // MARK: Actions

    private var selectedTask: TaskItem? {
        session.tasks.first { $0.id == ui.selectedTaskId }
    }

    private func toggleSelected() {
        guard let task = selectedTask else { return }
        session.setCompleted(task, task.completedAt == nil)
    }

    private func beginEdit() {
        guard let id = ui.selectedTaskId else { return }
        ui.editingTaskId = id
    }

    private func deleteSelected() {
        guard let task = selectedTask else { return }
        let list = rows
        let index = list.firstIndex { $0.id == task.id }
        session.delete(task)
        if let index, index + 1 < list.count {
            ui.selectedTaskId = list[index + 1].id
        } else if let index, index > 0 {
            ui.selectedTaskId = list[index - 1].id
        }
    }

    private func moveSelected(up: Bool) {
        guard let task = selectedTask else { return }
        session.move(task, up: up)
    }

    private func step(_ delta: Int) {
        let list = rows
        guard !list.isEmpty else { return }
        let index = list.firstIndex { $0.id == ui.selectedTaskId } ?? -1
        let next = min(max(index + delta, 0), list.count - 1)
        ui.selectedTaskId = list[next].id
    }

    /// Drag reorder among visible top-level rows of the same parent.
    private func moveRows(from source: IndexSet, to destination: Int) {
        guard let index = source.first else { return }
        let list = rows
        let moved = list[index].task
        var order = list
        order.remove(at: index)
        let target = destination > index ? destination - 1 : destination
        order.insert(list[index], at: target)
        let siblings = order.filter { $0.task.parentId == moved.parentId }
        guard let at = siblings.firstIndex(where: { $0.id == moved.id }) else { return }
        let after = at > 0 ? siblings[at - 1].task : nil
        let before = at + 1 < siblings.count ? siblings[at + 1].task : nil
        session.reorder(moved, after: after, before: before)
    }
}

struct TaskRowView: View {
    let row: Row
    let isEditing: Bool
    let isCollapsed: Bool
    var onToggle: @MainActor @Sendable (Bool) -> Void
    var onRename: @MainActor @Sendable (String) -> Void
    var onCancelEdit: @MainActor @Sendable () -> Void
    var onCollapse: @MainActor @Sendable () -> Void
    @State private var draft = ""
    @FocusState private var fieldFocused: Bool

    private var task: TaskItem { row.task }
    private var overdue: Bool { DueDisplay.isOverdue(task) }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: Tokens.Space.sm) {
            if row.hasChildren {
                Button(action: onCollapse) {
                    Image(systemName: isCollapsed ? "chevron.right" : "chevron.down")
                        .font(Tokens.Typography.caption)
                        .foregroundStyle(Tokens.Colors.textTertiary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(isCollapsed ? "Expand subtasks" : "Collapse subtasks")
            } else {
                Spacer().frame(width: Tokens.Space.md)
            }
            Toggle(isOn: Binding(get: { task.completedAt != nil }, set: onToggle)) { EmptyView() }
                .toggleStyle(.checkbox)
                .labelsHidden()
                .accessibilityLabel("Complete \(task.title)")
            VStack(alignment: .leading, spacing: Tokens.Space.xxs) {
                if isEditing {
                    TextField("Title", text: $draft)
                        .textFieldStyle(.plain)
                        .font(Tokens.Typography.body)
                        .focused($fieldFocused)
                        .onSubmit { onRename(draft) }
                        .onExitCommand { onCancelEdit() }
                        .onAppear {
                            draft = task.title
                            fieldFocused = true
                        }
                } else {
                    Text(task.title.isEmpty ? "Untitled" : task.title)
                        .font(Tokens.Typography.body)
                        .foregroundStyle(task.completedAt == nil ? Tokens.Colors.text : Tokens.Colors.textTertiary)
                        .strikethrough(task.completedAt != nil)
                }
                details
            }
            Spacer(minLength: 0)
        }
        .padding(.leading, CGFloat(row.depth) * Tokens.Space.xl)
        .padding(.vertical, Tokens.Space.xxs)
    }

    @ViewBuilder
    private var details: some View {
        let due = DueDisplay.text(for: task)
        if due != nil || task.listTitle != nil || !task.tags.isEmpty || task.priority != "none" || task.recurrence != nil {
            HStack(spacing: Tokens.Space.sm) {
                if let due {
                    Label(due, systemImage: overdue ? "exclamationmark.circle" : "calendar")
                        .foregroundStyle(overdue ? Tokens.Colors.overdue : Tokens.Colors.textSecondary)
                }
                if let list = task.listTitle {
                    Text("/\(list)").foregroundStyle(Tokens.Colors.spanList)
                }
                ForEach(task.tags, id: \.self) { tag in
                    Text("#\(tag)").foregroundStyle(Tokens.Colors.spanTag)
                }
                if task.priority != "none" {
                    Text("!\(task.priority)").foregroundStyle(Tokens.Colors.priority(task.priority))
                }
                if task.recurrence != nil {
                    Image(systemName: "repeat").foregroundStyle(Tokens.Colors.spanRecurrence)
                        .accessibilityLabel("Repeats")
                }
            }
            .font(Tokens.Typography.caption)
        }
    }
}
