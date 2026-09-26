// The kanban view: the same tasks as the list, grouped into columns by
// status, priority, list, or tag. Dropping a card on a column, or moving
// it with Option-arrows, changes that one field through the core. Column
// order and collapsed state are per-grouping preferences.

import ListeCore
import ListeKit
import SwiftUI
import UniformTypeIdentifiers

struct BoardView: View {
    @Bindable var session: Session
    @Bindable var ui: UIState
    @State private var group = Preferences.kanbanGroup
    @State private var collapsed: Set<String> = []
    @State private var order: [String] = []

    private struct Column: Identifiable {
        let key: String
        let title: String
        let tasks: [TaskItem]
        var id: String { key }
    }

    var body: some View {
        VStack(spacing: 0) {
            Picker("Group by", selection: $group) {
                Text("Status").tag("status")
                Text("Priority").tag("priority")
                Text("List").tag("list")
                Text("Tag").tag("tag")
            }
            .pickerStyle(.segmented)
            .padding(Tokens.Space.sm)
            .onChange(of: group) { _, g in
                Preferences.kanbanGroup = g
                load()
            }
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: Tokens.Space.md) {
                    ForEach(columns) { column in
                        columnView(column)
                    }
                }
                .padding(Tokens.Space.md)
            }
        }
        .onAppear(perform: load)
        .onKeyPress(.leftArrow, phases: .down) { press in
            guard press.modifiers.contains(.option) else { return .ignored }
            moveSelected(by: -1)
            return .handled
        }
        .onKeyPress(.rightArrow, phases: .down) { press in
            guard press.modifiers.contains(.option) else { return .ignored }
            moveSelected(by: 1)
            return .handled
        }
        .onKeyPress(.space) {
            if let task = session.tasks.first(where: { $0.id == ui.selectedTaskId }) {
                session.setCompleted(task, task.completedAt == nil)
            }
            return .handled
        }
    }

    private func load() {
        collapsed = Preferences.kanbanCollapsed(for: group)
        order = Preferences.kanbanColumnOrder(for: group)
    }

    /// Columns for the grouping, in the person's saved order with new keys
    /// appended, empty columns included so a card can be dropped anywhere.
    private var columns: [Column] {
        var keys: [(String, String)]
        var byKey: [String: [TaskItem]] = [:]
        switch group {
        case "priority":
            keys = [("high", "High"), ("medium", "Medium"), ("low", "Low"), ("none", "None")]
            for t in session.tasks { byKey[t.priority, default: []].append(t) }
        case "list":
            keys = [("", "Inbox")] + session.lists.map { ($0.id, $0.title) }
            for t in session.tasks { byKey[t.listId ?? "", default: []].append(t) }
        case "tag":
            keys = [("", "Untagged")] + session.tags.map { ($0.name, "#\($0.name)") }
            for t in session.tasks {
                if t.tags.isEmpty { byKey["", default: []].append(t) }
                for tag in t.tags { byKey[tag, default: []].append(t) }
            }
        default:
            var statuses = ["open", "doing", "done"]
            for t in session.tasks where !statuses.contains(t.status) { statuses.append(t.status) }
            keys = statuses.map { ($0, $0.capitalized) }
            for t in session.tasks { byKey[t.status, default: []].append(t) }
        }
        let saved = order
        let sorted = keys.sorted { a, b in
            let ia = saved.firstIndex(of: a.0) ?? Int.max
            let ib = saved.firstIndex(of: b.0) ?? Int.max
            return ia == ib ? false : ia < ib
        }
        return sorted.map { Column(key: $0.0, title: $0.1, tasks: byKey[$0.0] ?? []) }
    }

    private func columnView(_ column: Column) -> some View {
        let isCollapsed = collapsed.contains(column.key)
        return VStack(alignment: .leading, spacing: Tokens.Space.sm) {
            HStack {
                Button {
                    if isCollapsed { collapsed.remove(column.key) } else { collapsed.insert(column.key) }
                    Preferences.setKanbanCollapsed(collapsed, for: group)
                } label: {
                    Image(systemName: isCollapsed ? "chevron.right" : "chevron.down")
                        .font(Tokens.Typography.caption)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(isCollapsed ? "Expand \(column.title)" : "Collapse \(column.title)")
                Text(column.title).font(Tokens.Typography.bodyStrong)
                Text("\(column.tasks.count)").font(Tokens.Typography.caption).foregroundStyle(Tokens.Colors.textTertiary)
                Spacer()
                Button { shift(column.key, by: -1) } label: { Image(systemName: "arrow.left") }
                    .buttonStyle(.plain).accessibilityLabel("Move column left")
                Button { shift(column.key, by: 1) } label: { Image(systemName: "arrow.right") }
                    .buttonStyle(.plain).accessibilityLabel("Move column right")
            }
            if !isCollapsed {
                ScrollView {
                    LazyVStack(spacing: Tokens.Space.xs) {
                        ForEach(column.tasks, id: \.id) { task in
                            card(task)
                                .draggable(task.id)
                        }
                    }
                }
            }
        }
        .padding(Tokens.Space.sm)
        .frame(width: isCollapsed ? 180 : 260, alignment: .top)
        .background(Tokens.Colors.backgroundSecondary, in: RoundedRectangle(cornerRadius: Tokens.Radius.lg))
        .dropDestination(for: String.self) { ids, _ in
            for id in ids {
                if let task = session.tasks.first(where: { $0.id == id }) {
                    assign(task, to: column.key)
                }
            }
            return true
        }
    }

    private func card(_ task: TaskItem) -> some View {
        let selected = ui.selectedTaskId == task.id
        return VStack(alignment: .leading, spacing: Tokens.Space.xxs) {
            HStack(alignment: .firstTextBaseline, spacing: Tokens.Space.xs) {
                Toggle(isOn: Binding(get: { task.completedAt != nil }, set: { session.setCompleted(task, $0) })) { EmptyView() }
                    .toggleStyle(.checkbox).labelsHidden()
                Text(task.title.isEmpty ? "Untitled" : task.title)
                    .font(Tokens.Typography.body)
                    .strikethrough(task.completedAt != nil)
                    .lineLimit(3)
            }
            HStack(spacing: Tokens.Space.sm) {
                if let due = DueDisplay.text(for: task) {
                    Text(due).foregroundStyle(DueDisplay.isOverdue(task) ? Tokens.Colors.overdue : Tokens.Colors.textSecondary)
                }
                ForEach(task.tags, id: \.self) { Text("#\($0)").foregroundStyle(Tokens.Colors.spanTag) }
                if task.priority != "none" { Text("!\(task.priority)").foregroundStyle(Tokens.Colors.priority(task.priority)) }
            }
            .font(Tokens.Typography.caption)
        }
        .padding(Tokens.Space.sm)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Tokens.Colors.background, in: RoundedRectangle(cornerRadius: Tokens.Radius.md))
        .overlay(RoundedRectangle(cornerRadius: Tokens.Radius.md).stroke(selected ? Tokens.Colors.accent : Tokens.Colors.separator, lineWidth: selected ? 2 : 1))
        .contentShape(Rectangle())
        .onTapGesture { ui.selectedTaskId = task.id }
    }

    /// Change the grouped field of `task` so it lands in column `key`.
    private func assign(_ task: TaskItem, to key: String) {
        switch group {
        case "priority": session.setPriority(task, key)
        case "list":
            let name = session.lists.first { $0.id == key }?.title ?? ""
            session.setList(task, name)
        case "tag":
            if key.isEmpty {
                for tag in task.tags { session.removeTag(task, tag) }
            } else {
                for tag in task.tags where tag != key { session.removeTag(task, tag) }
                if !task.tags.contains(key) { session.addTag(task, key) }
            }
        default: session.setStatus(task, key)
        }
    }

    private func moveSelected(by delta: Int) {
        guard let task = session.tasks.first(where: { $0.id == ui.selectedTaskId }) else { return }
        let cols = columns
        guard let current = cols.firstIndex(where: { $0.tasks.contains { $0.id == task.id } }) else { return }
        let target = current + delta
        guard cols.indices.contains(target) else { return }
        assign(task, to: cols[target].key)
    }

    private func shift(_ key: String, by delta: Int) {
        var keys = columns.map(\.key)
        guard let i = keys.firstIndex(of: key), keys.indices.contains(i + delta) else { return }
        keys.swapAt(i, i + delta)
        order = keys
        Preferences.setKanbanColumnOrder(keys, for: group)
    }
}
