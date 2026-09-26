// The kanban view: the same tasks as the list, grouped into columns by
// status, priority, list, or tag. Each column is its own core query: a
// count, then pages of cards fetched as they scroll into view. Dropping a
// card on a column, or moving it with Option-arrows, changes that one
// field through the core. Column order and collapsed state are
// per-grouping preferences. A column is just its heading and its cards;
// only the cards have a surface.

import ListeCore
import ListeKit
import SwiftUI
import UniformTypeIdentifiers

/// One column's window: the count and the pages of cards loaded so far.
@MainActor
@Observable
final class BoardColumn: Identifiable {
    let key: String
    let title: String
    let query: QuerySpec
    private(set) var count = 0
    private var pages: [Int: [TaskItem]] = [:]
    private let pageSize = 50
    nonisolated var id: String { key }

    init(key: String, title: String, query: QuerySpec) {
        self.key = key
        self.title = title
        self.query = query
    }

    /// Re-count and re-fetch the pages already on screen.
    func reload(from session: Session) {
        count = session.count(query.query)
        let shown = pages.keys.filter { $0 * pageSize < count }
        pages = [:]
        for page in shown {
            pages[page] = session.fetch(query, offset: page * pageSize, limit: pageSize)
        }
    }

    func task(at index: Int) -> TaskItem? {
        let page = index / pageSize
        guard let rows = pages[page] else { return nil }
        let i = index - page * pageSize
        return rows.indices.contains(i) ? rows[i] : nil
    }

    /// Load the page holding `index` if it is not loaded yet.
    func ensure(_ index: Int, from session: Session) {
        let page = index / pageSize
        guard pages[page] == nil else { return }
        pages[page] = session.fetch(query, offset: page * pageSize, limit: pageSize)
    }

    func contains(_ id: String) -> Bool {
        pages.values.contains { $0.contains { $0.id == id } }
    }
}

struct BoardView: View {
    @Bindable var session: Session
    @Bindable var ui: UIState
    @State private var group = Preferences.kanbanGroup
    @State private var collapsed: Set<String> = []
    @State private var order: [String] = []
    @State private var columns: [BoardColumn] = []

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
        .onChange(of: session.generation) { _, _ in reloadColumns() }
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
            if let task = session.find(ui.selectedTaskId) {
                session.setCompleted(task, task.completedAt == nil)
            }
            return .handled
        }
    }

    private func load() {
        collapsed = Preferences.kanbanCollapsed(for: group)
        order = Preferences.kanbanColumnOrder(for: group)
        columns = makeColumns()
        for column in columns {
            column.reload(from: session)
        }
    }

    /// After a change: the same columns re-count, unless the set of
    /// columns itself changed (a new list, tag, or status).
    private func reloadColumns() {
        let fresh = makeColumns()
        if fresh.map(\.key) != columns.map(\.key) {
            columns = fresh
        }
        for column in columns {
            column.reload(from: session)
        }
    }

    /// Columns for the grouping, in the person's saved order with new keys
    /// appended, empty columns included so a card can be dropped anywhere.
    private func makeColumns() -> [BoardColumn] {
        let base = session.spec(for: session.selection)
        var keys: [(String, String, QuerySpec)]
        switch group {
        case "priority":
            keys = [("high", "High"), ("medium", "Medium"), ("low", "Low"), ("none", "None")].map { key, title in
                var q = base
                q.priority = key
                return (key, title, q)
            }
        case "list":
            var inbox = base
            inbox.inbox = true
            inbox.listId = nil
            keys = [("", "Inbox", inbox)]
            for list in session.lists {
                var q = base
                q.inbox = false
                q.listId = list.id
                keys.append((list.id, list.title, q))
            }
        case "tag":
            var untagged = base
            untagged.untagged = true
            untagged.tagId = nil
            keys = [("", "Untagged", untagged)]
            for tag in session.tags {
                var q = base
                q.tagId = tag.id
                keys.append((tag.name, "#\(tag.name)", q))
            }
        default:
            var statuses = ["open", "doing", "done"]
            for s in session.statuses() where !statuses.contains(s) { statuses.append(s) }
            keys = statuses.map { status in
                var q = base
                q.status = status
                return (status, status.capitalized, q)
            }
        }
        let saved = order
        let sorted = keys.sorted { a, b in
            let ia = saved.firstIndex(of: a.0) ?? Int.max
            let ib = saved.firstIndex(of: b.0) ?? Int.max
            return ia == ib ? false : ia < ib
        }
        return sorted.map { BoardColumn(key: $0.0, title: $0.1, query: $0.2) }
    }

    private func columnView(_ column: BoardColumn) -> some View {
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
                Text("\(column.count)").font(Tokens.Typography.caption).foregroundStyle(Tokens.Colors.textTertiary)
                Spacer()
                Button { shift(column.key, by: -1) } label: { Image(systemName: "arrow.left") }
                    .buttonStyle(.plain).accessibilityLabel("Move column left")
                Button { shift(column.key, by: 1) } label: { Image(systemName: "arrow.right") }
                    .buttonStyle(.plain).accessibilityLabel("Move column right")
            }
            if !isCollapsed {
                ScrollView {
                    LazyVStack(spacing: Tokens.Space.xs) {
                        ForEach(0..<column.count, id: \.self) { index in
                            cardSlot(column, index: index)
                        }
                    }
                }
            }
        }
        .frame(width: isCollapsed ? 180 : 260, alignment: .top)
        .contentShape(Rectangle())
        .dropDestination(for: String.self) { ids, _ in
            for id in ids {
                if let task = session.find(id) {
                    assign(task, to: column.key)
                }
            }
            return true
        }
    }

    /// A card, or an empty slot of the card's size until its page loads.
    @ViewBuilder
    private func cardSlot(_ column: BoardColumn, index: Int) -> some View {
        if let task = column.task(at: index) {
            card(task).draggable(task.id)
        } else {
            RoundedRectangle(cornerRadius: Tokens.Radius.md)
                .fill(Tokens.Colors.backgroundSecondary)
                .frame(height: Tokens.Size.row)
                .onAppear { column.ensure(index, from: session) }
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
        .background(Tokens.Colors.backgroundSecondary, in: RoundedRectangle(cornerRadius: Tokens.Radius.md))
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
        guard let task = session.find(ui.selectedTaskId) else { return }
        guard let current = columns.firstIndex(where: { $0.contains(task.id) }) else { return }
        let target = current + delta
        guard columns.indices.contains(target) else { return }
        assign(task, to: columns[target].key)
    }

    private func shift(_ key: String, by delta: Int) {
        var keys = columns.map(\.key)
        guard let i = keys.firstIndex(of: key), keys.indices.contains(i + delta) else { return }
        keys.swapAt(i, i + delta)
        order = keys
        Preferences.setKanbanColumnOrder(keys, for: group)
        columns = makeColumns()
        for column in columns { column.reload(from: session) }
    }
}
