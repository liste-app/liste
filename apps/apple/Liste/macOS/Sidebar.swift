// The sidebar: smart lists, the person's lists, tags, and saved filters.

import ListeCore
import ListeKit
import SwiftUI

struct Sidebar: View {
    @Bindable var session: Session
    @Bindable var ui: UIState
    @State private var renaming: TaskList?
    @State private var renameText = ""
    @State private var newListText = ""
    @State private var showNewList = false

    var body: some View {
        List(selection: $session.selection) {
            smartLists
            listsSection
            tagsSection
            filtersSection
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .bottom) { newListBar }
        .onChange(of: ui.newListRequest) { _, _ in showNewList = true }
        .sheet(item: $renaming) { list in renameSheet(list) }
    }

    private var newListBar: some View {
        HStack {
            Button {
                showNewList = true
            } label: {
                Label("New List", systemImage: "plus")
            }
            .buttonStyle(.borderless)
            .popover(isPresented: $showNewList) { newListPopover }
            Spacer()
        }
        .padding(Tokens.Space.sm)
    }

    private var newListPopover: some View {
        TextField("List name", text: $newListText)
            .frame(width: 220)
            .padding(Tokens.Space.md)
            .onSubmit(commitNewList)
    }

    private func commitNewList() {
        let name = newListText.trimmingCharacters(in: .whitespaces)
        if !name.isEmpty, let list = session.createList(name) {
            session.selection = .list(id: list.id)
        }
        newListText = ""
        showNewList = false
    }

    private func renameSheet(_ list: TaskList) -> some View {
        VStack(spacing: Tokens.Space.md) {
            Text("Rename List").font(Tokens.Typography.title)
            TextField("Name", text: $renameText)
                .onSubmit { commitRename(list) }
            HStack {
                Spacer()
                Button("Cancel") { renaming = nil }.keyboardShortcut(.cancelAction)
                Button("Rename") { commitRename(list) }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(Tokens.Space.xl)
        .frame(width: 320)
    }

    private var smartLists: some View {
        Section {
            row("Today", "sun.max", .today, badge: session.overdueCount)
            row("Upcoming", "calendar", .upcoming)
            row("Anytime", "tray.full", .anytime)
            row("Inbox", "tray", .inbox)
            row("Completed", "checkmark.circle", .completed)
        }
    }

    private var listsSection: some View {
        Section("Lists") {
            ForEach(session.lists, id: \.id) { list in
                Label(list.title, systemImage: "list.bullet")
                    .tag(Selection.list(id: list.id))
                    .contextMenu {
                        Button("Rename\u{2026}") { beginRename(list) }
                        Button("Delete List", role: .destructive) { session.deleteList(list) }
                    }
            }
            .onMove(perform: moveLists)
        }
    }

    @ViewBuilder
    private var tagsSection: some View {
        if !session.tags.isEmpty {
            Section("Tags") {
                ForEach(session.tags, id: \.id) { tag in
                    Label(tag.name, systemImage: "number").tag(Selection.tag(id: tag.id))
                }
            }
        }
    }

    /// Saved filters come from the core and sync like lists.
    @ViewBuilder
    private var filtersSection: some View {
        if !session.filters.isEmpty {
            Section("Filters") {
                ForEach(session.filters) { filter in
                    Label(filter.name, systemImage: "line.3.horizontal.decrease.circle")
                        .tag(Selection.filter(id: filter.id))
                        .contextMenu {
                            Button("Delete Filter", role: .destructive) { session.deleteFilter(filter) }
                        }
                }
                .onMove(perform: moveFilters)
            }
        }
    }

    private func moveFilters(from source: IndexSet, to destination: Int) {
        guard let index = source.first else { return }
        let filters = session.filters
        let moved = filters[index]
        var order = filters
        order.remove(at: index)
        let target = destination > index ? destination - 1 : destination
        order.insert(moved, at: target)
        let after = target > 0 ? order[target - 1] : nil
        let before = target + 1 < order.count ? order[target + 1] : nil
        session.reorderFilter(moved, after: after, before: before)
    }

    private func row(_ title: String, _ icon: String, _ selection: Selection, badge: Int = 0) -> some View {
        Label(title, systemImage: icon)
            .badge(badge > 0 ? badge : 0)
            .tag(selection)
    }

    private func beginRename(_ list: TaskList) {
        renameText = list.title
        renaming = list
    }

    private func commitRename(_ list: TaskList) {
        let name = renameText.trimmingCharacters(in: .whitespaces)
        if !name.isEmpty { session.renameList(list, to: name) }
        renaming = nil
    }

    private func moveLists(from source: IndexSet, to destination: Int) {
        guard let index = source.first else { return }
        let lists = session.lists
        let moved = lists[index]
        var order = lists
        order.remove(at: index)
        let target = destination > index ? destination - 1 : destination
        order.insert(moved, at: target)
        let after = target > 0 ? order[target - 1] : nil
        let before = target + 1 < order.count ? order[target + 1] : nil
        session.reorderList(moved, after: after, before: before)
    }
}
