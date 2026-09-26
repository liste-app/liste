// The task inspector: every field of the selected task. Each change is one
// update through the session; text fields commit on submit or focus loss.

import ListeCore
import ListeKit
import SwiftUI

struct Inspector: View {
    @Bindable var session: Session
    @Bindable var ui: UIState

    var body: some View {
        if let task = session.find(ui.selectedTaskId) {
            InspectorForm(task: task, session: session)
                .id(task.id)
        } else {
            ContentUnavailableView("No Selection", systemImage: "sidebar.right", description: Text("Select a task to see its details."))
        }
    }
}

struct InspectorForm: View {
    let task: TaskItem
    @Bindable var session: Session
    @State private var title: String
    @State private var notes: String
    @State private var due: String
    @State private var reminder: String
    @State private var newTag = ""

    init(task: TaskItem, session: Session) {
        self.task = task
        self.session = session
        _title = State(initialValue: task.title)
        _notes = State(initialValue: task.notes)
        _due = State(initialValue: task.due ?? "")
        _reminder = State(initialValue: task.reminderAt.map(DueDisplay.absolute) ?? "")
    }

    var body: some View {
        Form {
            Section {
                TextField("Title", text: $title)
                    .font(Tokens.Typography.bodyStrong)
                    .onSubmit { session.rename(task, to: title) }
                Toggle("Completed", isOn: Binding(get: { task.completedAt != nil }, set: { session.setCompleted(task, $0) }))
            }
            Section("Schedule") {
                TextField("Due", text: $due, prompt: Text("tomorrow 5pm, next friday, jan 5"))
                    .onSubmit { session.setDue(task, due) }
                if let text = DueDisplay.text(for: task) {
                    LabeledContent("Resolves to", value: text)
                        .foregroundStyle(DueDisplay.isOverdue(task) ? Tokens.Colors.overdue : Tokens.Colors.textSecondary)
                }
                TextField("Reminder", text: $reminder, prompt: Text("in 1 hour, tomorrow 9am"))
                    .onSubmit { session.setReminder(task, reminder) }
                Picker("Repeats", selection: Binding(get: { task.recurrence ?? "" }, set: { session.update(task, TaskPatch(due: recurrenceText($0))) })) {
                    Text("Never").tag("")
                    Text("Every day").tag("FREQ=DAILY")
                    Text("Every weekday").tag("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")
                    Text("Every week").tag("FREQ=WEEKLY")
                    Text("Every month").tag("FREQ=MONTHLY")
                    Text("Every year").tag("FREQ=YEARLY")
                    if let rule = task.recurrence, !["", "FREQ=DAILY", "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR", "FREQ=WEEKLY", "FREQ=MONTHLY", "FREQ=YEARLY"].contains(rule) {
                        Text(rule).tag(rule)
                    }
                }
            }
            Section("Organize") {
                Picker("List", selection: Binding(get: { task.listTitle ?? "" }, set: { session.setList(task, $0) })) {
                    Text("Inbox").tag("")
                    ForEach(session.lists, id: \.id) { list in
                        Text(list.title).tag(list.title)
                    }
                }
                Picker("Priority", selection: Binding(get: { task.priority }, set: { session.setPriority(task, $0) })) {
                    Text("None").tag("none")
                    Text("Low").tag("low")
                    Text("Medium").tag("medium")
                    Text("High").tag("high")
                }
                Picker("Status", selection: Binding(get: { task.status }, set: { session.setStatus(task, $0) })) {
                    ForEach(statuses, id: \.self) { s in
                        Text(s.capitalized).tag(s)
                    }
                }
                LabeledContent("Tags") {
                    VStack(alignment: .leading, spacing: Tokens.Space.xs) {
                        ForEach(task.tags, id: \.self) { tag in
                            HStack {
                                Text("#\(tag)").foregroundStyle(Tokens.Colors.spanTag)
                                Spacer()
                                Button { session.removeTag(task, tag) } label: {
                                    Image(systemName: "xmark.circle.fill").foregroundStyle(Tokens.Colors.textTertiary)
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel("Remove tag \(tag)")
                            }
                        }
                        TextField("Add tag", text: $newTag)
                            .onSubmit {
                                let name = newTag.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "#"))
                                if !name.isEmpty { session.addTag(task, name) }
                                newTag = ""
                            }
                    }
                }
                // Candidates are the loaded top-level rows, plus the current
                // parent so the picker always shows it.
                Picker("Parent", selection: Binding(get: { task.parentId ?? "" }, set: { session.setParent(task, $0.isEmpty ? nil : $0) })) {
                    Text("None").tag("")
                    if let parentId = task.parentId, !session.tasks.contains(where: { $0.id == parentId }),
                        let parent = session.find(parentId)
                    {
                        Text(parent.title).tag(parent.id)
                    }
                    ForEach(session.tasks.filter { $0.id != task.id && $0.parentId == nil }, id: \.id) { candidate in
                        Text(candidate.title).tag(candidate.id)
                    }
                }
            }
            Section("Notes") {
                TextEditor(text: $notes)
                    .font(Tokens.Typography.body)
                    .frame(minHeight: 120)
                    .onChange(of: notes) { _, value in
                        // Notes commit when the person pauses, as one op.
                        notesCommit = Task { @MainActor in
                            try? await Task.sleep(for: .milliseconds(600))
                            if !Task.isCancelled { session.setNotes(task, value) }
                        }
                    }
            }
            let subtasks = task.hasSubtasks ? session.subtasks(of: task) : []
            if !subtasks.isEmpty {
                Section("Subtasks") {
                    ForEach(subtasks, id: \.id) { sub in
                        Toggle(sub.title, isOn: Binding(get: { sub.completedAt != nil }, set: { session.setCompleted(sub, $0) }))
                    }
                }
            }
            Section("Dates") {
                LabeledContent("Created", value: DueDisplay.absolute(task.createdAt))
                if let done = task.completedAt {
                    LabeledContent("Completed", value: DueDisplay.absolute(done))
                }
                LabeledContent("Modified", value: DueDisplay.absolute(task.modifiedAt))
            }
        }
        .formStyle(.grouped)
        .onDisappear {
            notesCommit?.cancel()
            session.setNotes(task, notes)
            if title != task.title { session.rename(task, to: title) }
        }
    }

    @State private var notesCommit: Task<Void, Never>?

    private var statuses: [String] {
        var all = ["open", "doing", "done"]
        if !all.contains(task.status) { all.append(task.status) }
        return all
    }

    /// The recurrence picker sets the rule through the natural-language
    /// due field, which is the one path that also anchors a first date.
    private func recurrenceText(_ rule: String) -> String {
        let phrase: String = switch rule {
        case "FREQ=DAILY": "every day"
        case "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR": "every weekday"
        case "FREQ=WEEKLY": "every week"
        case "FREQ=MONTHLY": "every month"
        case "FREQ=YEARLY": "every year"
        default: ""
        }
        if phrase.isEmpty { return due.isEmpty ? "" : due }
        return due.isEmpty ? phrase : "\(due) \(phrase)"
    }
}
