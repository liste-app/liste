// The main window: sidebar, the list or board of the selected item, a
// search field, and the inspector. Every value shown comes from the core
// through the session.

import ListeCore
import ListeKit
import SwiftUI

struct MainView: View {
    @Bindable var session: Session
    @Bindable var ui: UIState
    @FocusState private var searchFocused: Bool

    var body: some View {
        NavigationSplitView {
            Sidebar(session: session, ui: ui)
                .navigationSplitViewColumnWidth(min: 180, ideal: 220)
        } detail: {
            content
                .toolbar { toolbar }
                .navigationTitle(title)
                .navigationSubtitle(subtitle)
        }
        .inspector(isPresented: $ui.showInspector) {
            Inspector(session: session, ui: ui)
                .inspectorColumnWidth(min: 260, ideal: 320)
        }
        .onChange(of: ui.searchFocusRequest) { _, _ in
            if !session.selection.isSearch { session.beginSearch() }
            searchFocused = true
        }
        .onChange(of: ui.newTaskRequest) { _, _ in
            if let task = session.newTask() {
                ui.selectedTaskId = task.id
                ui.editingTaskId = task.id
            }
        }
        .onChange(of: ui.searchText) { _, text in
            if session.selection.isSearch { session.setSearch(text) }
        }
        .onExitCommand {
            if session.selection.isSearch {
                ui.searchText = ""
                session.endSearch()
            }
        }
        .onChange(of: ui.paletteRequest) { _, _ in ui.showPalette = true }
        .sheet(isPresented: $ui.showPalette) {
            CommandPalette(session: session, ui: ui, dismiss: { ui.showPalette = false }, runAction: runAction)
        }
        .sheet(isPresented: $ui.showFilterBuilder) {
            FilterBuilder(session: session, dismiss: { ui.showFilterBuilder = false })
        }
        .preferredColorScheme(scheme)
    }

    private func runAction(_ action: String) {
        switch action {
        case "new-task": ui.newTaskRequest += 1
        case "quick-capture": NotificationCenter.default.post(name: .quickCaptureRequested, object: nil)
        case "search": ui.searchFocusRequest += 1
        case "inspector": ui.showInspector.toggle()
        case "list-view": ui.viewMode = .list
        case "board-view": ui.viewMode = .board
        case "undo": session.undo()
        case "redo": session.redo()
        case "complete":
            if let task = session.tasks.first(where: { $0.id == ui.selectedTaskId }) {
                session.setCompleted(task, task.completedAt == nil)
            }
        case "delete":
            if let task = session.tasks.first(where: { $0.id == ui.selectedTaskId }) { session.delete(task) }
        case "new-filter": ui.showFilterBuilder = true
        case "settings": NotificationCenter.default.post(name: .settingsRequested, object: nil)
        default: break
        }
    }

    @ViewBuilder
    private var content: some View {
        switch ui.viewMode {
        case .list: TaskListView(session: session, ui: ui)
        case .board: BoardView(session: session, ui: ui)
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        ToolbarItem(placement: .principal) {
            Picker("View", selection: $ui.viewMode) {
                Image(systemName: "list.bullet").tag(UIState.ViewMode.list).help("List (\u{2318}\u{21E7}1)")
                Image(systemName: "rectangle.split.3x1").tag(UIState.ViewMode.board).help("Board (\u{2318}\u{21E7}2)")
            }
            .pickerStyle(.segmented)
            .labelsHidden()
        }
        ToolbarItem(placement: .automatic) {
            TextField("Search", text: $ui.searchText)
                .textFieldStyle(.roundedBorder)
                .frame(width: 200)
                .focused($searchFocused)
                .onSubmit { searchFocused = false }
                .onChange(of: searchFocused) { _, focused in
                    if focused, !session.selection.isSearch { session.beginSearch() }
                }
                .accessibilityLabel("Search tasks")
        }
        ToolbarItem(placement: .automatic) {
            Button { ui.newTaskRequest += 1 } label: { Label("New Task", systemImage: "plus") }
                .help("New Task (\u{2318}N)")
        }
        ToolbarItem(placement: .automatic) {
            Button { ui.showInspector.toggle() } label: { Label("Inspector", systemImage: "sidebar.right") }
                .help("Show Inspector (\u{2318}I)")
        }
    }

    private var title: String {
        switch session.selection {
        case .today: "Today"
        case .upcoming: "Upcoming"
        case .anytime: "Anytime"
        case .completed: "Completed"
        case .inbox: "Inbox"
        case .list(let id): session.lists.first { $0.id == id }?.title ?? "List"
        case .tag(let id): "#" + (session.tags.first { $0.id == id }?.name ?? "tag")
        case .search(let q): q.isEmpty ? "Search" : "Search: \(q)"
        case .filter(let f): f.name
        }
    }

    private var subtitle: String {
        let n = session.tasks.count
        return n == 1 ? "1 task" : "\(n) tasks"
    }

    private var scheme: ColorScheme? {
        switch Preferences.theme {
        case "light": .light
        case "dark": .dark
        default: nil
        }
    }
}

extension Notification.Name {
    static let quickCaptureRequested = Notification.Name("liste.quickCapture")
    static let settingsRequested = Notification.Name("liste.settings")
}
