// The kanban view: the same tasks as the list, grouped into columns by
// status, priority, list, or tag. Each column is its own core query;
// dropping a card between two cards of any column changes that one field
// and its place in manual order through the core, in one change, and
// Option-arrows move it to the next column. Column order is a per-grouping
// preference, set by dragging a heading. The columns themselves are
// AppKit tables (see Board.swift).

import ListeCore
import ListeKit
import SwiftUI

struct BoardView: View {
    @Bindable var session: Session
    @Bindable var ui: UIState
    @State private var group = Preferences.kanbanGroup
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
            Board(session: session, ui: ui, columns: columns, actions: actions)
        }
        .onAppear(perform: load)
        .onChange(of: session.generation) { _, _ in reloadColumns() }
    }

    private var actions: BoardActions {
        BoardActions(
            move: { task, key, after, before in move(task, to: key, after: after, before: before) },
            reorderColumns: { keys in
                order = keys
                Preferences.setKanbanColumnOrder(keys, for: group)
                columns = makeColumns()
                for column in columns { column.reload(from: session) }
            },
            select: { id in ui.selectedTaskId = id },
            toggleCompleted: { task in session.setCompleted(task, task.completedAt == nil) }
        )
    }

    private func load() {
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

    /// Change the grouped field of `task` so it lands in column `key`, and
    /// its place in manual order, as one change and one undo step.
    private func move(_ task: TaskItem, to key: String, after: TaskItem?, before: TaskItem?) {
        var priority: String?
        var list: String?
        var addTags: [String] = []
        var removeTags: [String] = []
        var status: String?
        switch group {
        case "priority":
            if task.priority != key { priority = key }
        case "list":
            if (task.listId ?? "") != key { list = session.lists.first { $0.id == key }?.title ?? "" }
        case "tag":
            removeTags = key.isEmpty ? task.tags : task.tags.filter { $0 != key }
            if !key.isEmpty, !task.tags.contains(key) { addTags = [key] }
        default:
            if task.status != key { status = key }
        }
        session.update(
            task,
            TaskPatch(
                priority: priority, list: list, addTags: addTags, removeTags: removeTags, status: status,
                after: after?.id, before: before?.id))
    }
}
