// The command palette (⌘K): actions, places, lists, tags, and tasks by
// fuzzy match. Matching is presentation; every chosen action goes through
// the session.

import ListeCore
import ListeKit
import SwiftUI

struct PaletteItem: Identifiable, Hashable {
    enum Kind { case action, place, list, tag, task }
    let id: String
    let kind: Kind
    let title: String
    let subtitle: String?
    let icon: String
}

/// Subsequence match with a simple score: shorter gaps and word-start hits
/// rank higher.
func fuzzyScore(_ needle: String, in haystack: String) -> Int? {
    let n = Array(needle.lowercased())
    let h = Array(haystack.lowercased())
    guard !n.isEmpty else { return 0 }
    var score = 0
    var hi = 0
    var lastHit = -1
    for c in n {
        guard let found = h[hi...].firstIndex(of: c) else { return nil }
        score += found == lastHit + 1 ? 3 : 1
        if found == 0 || h[found - 1] == " " { score += 2 }
        lastHit = found
        hi = found + 1
    }
    return score - (h.count - n.count) / 10
}

struct CommandPalette: View {
    @Bindable var session: Session
    @Bindable var ui: UIState
    var dismiss: () -> Void
    var runAction: (String) -> Void
    @State private var query = ""
    @State private var highlighted = 0
    @FocusState private var focused: Bool

    private var actions: [PaletteItem] {
        [
            ("new-task", "New Task", "plus", "\u{2318}N"),
            ("quick-capture", "Quick Capture", "text.cursor", "\u{2325}Space"),
            ("search", "Search", "magnifyingglass", "\u{2318}F"),
            ("inspector", "Toggle Inspector", "sidebar.right", "\u{2318}I"),
            ("list-view", "List View", "list.bullet", "\u{2318}\u{21E7}1"),
            ("board-view", "Board View", "rectangle.split.3x1", "\u{2318}\u{21E7}2"),
            ("undo", "Undo", "arrow.uturn.backward", "\u{2318}Z"),
            ("redo", "Redo", "arrow.uturn.forward", "\u{2318}\u{21E7}Z"),
            ("complete", "Complete Selected", "checkmark.circle", "Space"),
            ("delete", "Delete Selected", "trash", "\u{232B}"),
            ("new-filter", "New Saved Filter\u{2026}", "line.3.horizontal.decrease.circle", nil),
            ("settings", "Settings\u{2026}", "gearshape", "\u{2318},"),
        ].map { PaletteItem(id: "action:\($0.0)", kind: .action, title: $0.1, subtitle: $0.3, icon: $0.2) }
    }

    private var places: [PaletteItem] {
        [("today", "Today", "sun.max"), ("upcoming", "Upcoming", "calendar"), ("anytime", "Anytime", "tray.full"),
         ("inbox", "Inbox", "tray"), ("completed", "Completed", "checkmark.circle")]
            .map { PaletteItem(id: "place:\($0.0)", kind: .place, title: $0.1, subtitle: "Go to", icon: $0.2) }
    }

    private var results: [PaletteItem] {
        let all = actions + places
            + session.lists.map { PaletteItem(id: "list:\($0.id)", kind: .list, title: $0.title, subtitle: "List", icon: "list.bullet") }
            + session.tags.map { PaletteItem(id: "tag:\($0.id)", kind: .tag, title: "#\($0.name)", subtitle: "Tag", icon: "number") }
            + (query.isEmpty ? [] : session.search(query, limit: 20).map {
                PaletteItem(id: "task:\($0.id)", kind: .task, title: $0.title, subtitle: $0.listTitle.map { "/\($0)" } ?? "Task", icon: "circle")
            })
        if query.isEmpty { return Array(all.prefix(12)) }
        return all.compactMap { item in fuzzyScore(query, in: item.title).map { (item, $0) } }
            .sorted { $0.1 > $1.1 }
            .prefix(20)
            .map(\.0)
    }

    var body: some View {
        VStack(spacing: 0) {
            TextField("Type a command, list, tag, or task", text: $query)
                .textFieldStyle(.plain)
                .font(Tokens.Typography.capture)
                .padding(Tokens.Space.lg)
                .focused($focused)
                .onSubmit { run(results.indices.contains(highlighted) ? results[highlighted] : nil) }
                .onChange(of: query) { _, _ in highlighted = 0 }
            Divider()
            ScrollViewReader { proxy in
                List(Array(results.enumerated()), id: \.element.id) { index, item in
                    HStack {
                        Image(systemName: item.icon).frame(width: 20)
                        Text(item.title).font(Tokens.Typography.body)
                        Spacer()
                        if let sub = item.subtitle {
                            Text(sub).font(Tokens.Typography.caption).foregroundStyle(Tokens.Colors.textTertiary)
                        }
                    }
                    .padding(.vertical, Tokens.Space.xxs)
                    .listRowBackground(index == highlighted ? Tokens.Colors.selection : Color.clear)
                    .contentShape(Rectangle())
                    .onTapGesture { run(item) }
                    .id(item.id)
                }
                .listStyle(.plain)
                .frame(height: 320)
                .onChange(of: highlighted) { _, h in
                    if results.indices.contains(h) { proxy.scrollTo(results[h].id) }
                }
            }
        }
        .frame(width: 560)
        .onAppear { focused = true }
        .onKeyPress(.downArrow) { highlighted = min(highlighted + 1, max(results.count - 1, 0)); return .handled }
        .onKeyPress(.upArrow) { highlighted = max(highlighted - 1, 0); return .handled }
        .onExitCommand { dismiss() }
    }

    private func run(_ item: PaletteItem?) {
        guard let item else { return }
        switch item.kind {
        case .action: runAction(String(item.id.dropFirst("action:".count)))
        case .place:
            session.selection = switch item.id {
            case "place:upcoming": .upcoming
            case "place:anytime": .anytime
            case "place:inbox": .inbox
            case "place:completed": .completed
            default: .today
            }
        case .list: session.selection = .list(id: String(item.id.dropFirst("list:".count)))
        case .tag: session.selection = .tag(id: String(item.id.dropFirst("tag:".count)))
        case .task:
            let id = String(item.id.dropFirst("task:".count))
            if let task = try? session.task(id: id) {
                session.selection = task.listId.map { .list(id: $0) } ?? .inbox
                ui.selectedTaskId = id
                ui.showInspector = true
            }
        }
        dismiss()
    }
}

/// The saved-filter builder: list, tag, priority, due range, status.
struct FilterBuilder: View {
    @Bindable var session: Session
    var dismiss: () -> Void
    @State private var filter = SavedFilter(name: "")

    var body: some View {
        Form {
            TextField("Name", text: $filter.name)
            Picker("List", selection: Binding(get: { filter.listId ?? "" }, set: { filter.listId = $0.isEmpty ? nil : $0 })) {
                Text("Any").tag("")
                ForEach(session.lists, id: \.id) { Text($0.title).tag($0.id) }
            }
            Picker("Tag", selection: Binding(get: { filter.tagId ?? "" }, set: { filter.tagId = $0.isEmpty ? nil : $0 })) {
                Text("Any").tag("")
                ForEach(session.tags, id: \.id) { Text("#\($0.name)").tag($0.id) }
            }
            Picker("Priority", selection: Binding(get: { filter.priority ?? "" }, set: { filter.priority = $0.isEmpty ? nil : $0 })) {
                Text("Any").tag("")
                ForEach(["high", "medium", "low", "none"], id: \.self) { Text($0.capitalized).tag($0) }
            }
            Picker("Due", selection: $filter.dueWithinDays) {
                Text("Any").tag(0)
                Text("Overdue").tag(-1)
                Text("Next 7 days").tag(7)
                Text("Next 30 days").tag(30)
            }
            Picker("Status", selection: Binding(get: { filter.status ?? "" }, set: { filter.status = $0.isEmpty ? nil : $0 })) {
                Text("Any").tag("")
                ForEach(["open", "doing", "done"], id: \.self) { Text($0.capitalized).tag($0) }
            }
            Toggle("Include completed", isOn: $filter.includeCompleted)
            HStack {
                Spacer()
                Button("Cancel", action: dismiss).keyboardShortcut(.cancelAction)
                Button("Save") {
                    guard !filter.name.trimmingCharacters(in: .whitespaces).isEmpty else { return }
                    var all = Preferences.savedFilters
                    all.append(filter)
                    Preferences.savedFilters = all
                    NotificationCenter.default.post(name: .savedFiltersChanged, object: nil)
                    session.selection = .filter(filter)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .formStyle(.grouped)
        .frame(width: 380)
        .padding(Tokens.Space.md)
    }
}
