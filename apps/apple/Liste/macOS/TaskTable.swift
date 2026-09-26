// The task table: an NSTableView with rows of one fixed height, so a list
// of 40,000 tasks is laid out without measuring any of them (Section 4:
// virtualize long lists). The table asks the session for the rows it is
// about to show and the session fetches only those, plus a margin.
//
// Rows are AppKit controls, not hosted SwiftUI views, so a recycled cell
// is configured in microseconds and a fast scroll stays under a frame.
// Nothing here mutates the session from inside a table callback that runs
// during the table's own update pass. Rows the session has not loaded yet
// are drawn empty and requested on the next turn of the run loop.

import AppKit
import ListeCore
import ListeKit
import SwiftUI

struct TaskTable: NSViewRepresentable {
    let session: Session
    let ui: UIState
    /// Show a day column, with a heading on the first row of each day.
    let showsDayColumn: Bool

    func makeNSView(context: Context) -> NSScrollView {
        let table = TableView()
        table.coordinator = context.coordinator
        let column = NSTableColumn(identifier: .init("task"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)
        table.headerView = nil
        table.rowHeight = Tokens.Size.row
        table.usesAutomaticRowHeights = false
        table.style = .inset
        table.selectionHighlightStyle = .regular
        table.allowsMultipleSelection = false
        table.allowsEmptySelection = true
        table.allowsTypeSelect = false
        table.intercellSpacing = .zero
        table.columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle
        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        table.registerForDraggedTypes([Coordinator.dragType])
        table.setDraggingSourceOperationMask(.move, forLocal: true)
        table.target = context.coordinator
        table.doubleAction = #selector(Coordinator.doubleClicked)
        let menu = NSMenu()
        menu.delegate = context.coordinator
        table.menu = menu
        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.contentView.postsBoundsChangedNotifications = true
        context.coordinator.attach(table: table, clipView: scroll.contentView)
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        // Reading these here is what makes SwiftUI call again when they change.
        context.coordinator.apply(
            selection: session.selection,
            count: session.count,
            tasks: session.tasks,
            windowStart: session.windowStart,
            selected: ui.selectedTaskId,
            editing: ui.editingTaskId,
            collapsed: session.collapsed,
            showsDayColumn: showsDayColumn
        )
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(session: session, ui: ui)
    }

    /// The table fills whatever it is given; answering here keeps SwiftUI
    /// from measuring it, and re-laying out the window, on every update.
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSScrollView, context: Context) -> CGSize? {
        proposal.replacingUnspecifiedDimensions(by: CGSize(width: 320, height: 240))
    }

    static func dismantleNSView(_ nsView: NSScrollView, coordinator: Coordinator) {
        coordinator.detach()
    }

    /// The table's data source, delegate, and key handler. Its snapshot of
    /// the session's window is what the table reads; `apply` replaces it.
    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
        static let dragType = NSPasteboard.PasteboardType("com.example.liste.task-id")
        private static let cellId = NSUserInterfaceItemIdentifier("task-cell")

        let session: Session
        let ui: UIState
        private(set) weak var table: NSTableView?
        private var count = 0
        private var selection: Selection?
        private var tasks: [TaskItem] = []
        private var windowStart = 0
        private var editing: String?
        private var collapsed: Set<String> = []
        private var showsDayColumn = false
        private var syncingSelection = false
        private var loadRequested = false
        private var observer: NSObjectProtocol?

        init(session: Session, ui: UIState) {
            self.session = session
            self.ui = ui
        }

        func detach() {
            if let observer { NotificationCenter.default.removeObserver(observer) }
            observer = nil
        }

        func attach(table: NSTableView, clipView: NSClipView) {
            self.table = table
            observer = NotificationCenter.default.addObserver(
                forName: NSView.boundsDidChangeNotification, object: clipView, queue: .main
            ) { [weak self] _ in
                MainActor.assumeIsolated { self?.prefetchVisibleRows() }
            }
        }

        // MARK: Snapshot

        func apply(
            selection: Selection, count: Int, tasks: [TaskItem], windowStart: Int, selected: String?,
            editing: String?, collapsed: Set<String>, showsDayColumn: Bool
        ) {
            guard let table else { return }
            let countChanged = count != self.count
            self.tasks = tasks
            self.windowStart = windowStart
            self.editing = editing
            self.collapsed = collapsed
            self.showsDayColumn = showsDayColumn
            // Never `reloadData`: with tens of thousands of rows it walks
            // every row, while noting the count and refilling the rows on
            // screen costs only what is on screen.
            if countChanged {
                self.count = count
                table.noteNumberOfRowsChanged()
            }
            if selection != self.selection {
                self.selection = selection
                table.scroll(.zero)
            }
            reconfigureVisibleRows()
            syncSelection(to: selected)
            if let editing, let row = index(of: editing) {
                table.scrollRowToVisible(row)
            }
        }

        private func task(at row: Int) -> TaskItem? {
            let i = row - windowStart
            return tasks.indices.contains(i) ? tasks[i] : nil
        }

        private func index(of id: String) -> Int? {
            tasks.firstIndex { $0.id == id }.map { $0 + windowStart }
        }

        private var visibleRows: Range<Int> {
            guard let table else { return 0..<0 }
            let rows = table.rows(in: table.visibleRect)
            return rows.location..<(rows.location + rows.length)
        }

        /// Called as the clip view scrolls, before the table lays out the
        /// rows that came into view: load them now so no empty row shows.
        private func prefetchVisibleRows() {
            let rows = visibleRows
            guard !rows.isEmpty else { return }
            session.ensureLoaded(rows)
            tasks = session.tasks
            windowStart = session.windowStart
        }

        /// A row the snapshot lacks was drawn empty; fetch it on the next
        /// turn, outside the table's update.
        private func requestLoad() {
            guard !loadRequested else { return }
            loadRequested = true
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                loadRequested = false
                let rows = visibleRows
                session.ensureLoaded(rows)
                tasks = session.tasks
                windowStart = session.windowStart
                reconfigureVisibleRows()
            }
        }

        private func reconfigureVisibleRows() {
            guard let table else { return }
            for row in visibleRows {
                if let cell = table.view(atColumn: 0, row: row, makeIfNecessary: false) as? TaskCell {
                    configure(cell, row: row)
                }
            }
        }

        private func syncSelection(to id: String?) {
            guard let table else { return }
            let row = id.flatMap(index(of:))
            guard row != (table.selectedRow >= 0 ? table.selectedRow : nil) else { return }
            syncingSelection = true
            if let row {
                table.selectRowIndexes([row], byExtendingSelection: false)
            } else {
                table.deselectAll(nil)
            }
            syncingSelection = false
        }

        // MARK: Data source and delegate

        func numberOfRows(in tableView: NSTableView) -> Int {
            count
        }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            let cell = (tableView.makeView(withIdentifier: Self.cellId, owner: nil) as? TaskCell) ?? TaskCell(identifier: Self.cellId)
            configure(cell, row: row)
            return cell
        }

        private func configure(_ cell: TaskCell, row: Int) {
            guard let task = task(at: row) else {
                cell.showEmpty(dayColumn: showsDayColumn)
                requestLoad()
                return
            }
            var heading: String?
            if showsDayColumn {
                let day = DueDisplay.dayHeading(for: task.dueAt)
                let previous = row > 0 ? self.task(at: row - 1) : nil
                if row == 0 || previous.map({ DueDisplay.dayHeading(for: $0.dueAt) != day }) ?? true {
                    heading = day
                }
            }
            cell.show(
                task, dayColumn: showsDayColumn, dayHeading: heading,
                isEditing: editing == task.id, isCollapsed: collapsed.contains(task.id))
            cell.onToggle = { [session] done in session.setCompleted(task, done) }
            cell.onRename = { [session, ui] title in
                session.rename(task, to: title)
                ui.editingTaskId = nil
            }
            cell.onCancelEdit = { [ui] in ui.editingTaskId = nil }
            cell.onCollapse = { [session] in
                if session.collapsed.contains(task.id) {
                    session.collapsed.remove(task.id)
                } else {
                    session.collapsed.insert(task.id)
                }
            }
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            guard !syncingSelection, let table else { return }
            let id = table.selectedRow >= 0 ? task(at: table.selectedRow)?.id : nil
            if ui.selectedTaskId != id {
                ui.selectedTaskId = id
            }
        }

        func tableView(_ tableView: NSTableView, shouldTypeSelectFor event: NSEvent, withCurrentSearch searchString: String?) -> Bool {
            false
        }

        // MARK: Drag reorder

        func tableView(_ tableView: NSTableView, pasteboardWriterForRow row: Int) -> NSPasteboardWriting? {
            guard let task = task(at: row) else { return nil }
            let item = NSPasteboardItem()
            item.setString(task.id, forType: Self.dragType)
            return item
        }

        func tableView(
            _ tableView: NSTableView, validateDrop info: NSDraggingInfo, proposedRow row: Int,
            proposedDropOperation dropOperation: NSTableView.DropOperation
        ) -> NSDragOperation {
            guard info.draggingSource as? NSTableView === tableView else { return [] }
            if dropOperation == .on {
                tableView.setDropRow(row, dropOperation: .above)
            }
            return .move
        }

        func tableView(
            _ tableView: NSTableView, acceptDrop info: NSDraggingInfo, row: Int,
            dropOperation: NSTableView.DropOperation
        ) -> Bool {
            guard let id = info.draggingPasteboard.string(forType: Self.dragType),
                let moved = task(at: index(of: id) ?? -1)
            else { return false }
            // The new neighbours are the nearest rows above and below the
            // drop point with the same parent as the moved task.
            var after: TaskItem?
            var i = row - 1
            while i >= 0, let t = task(at: i) {
                if t.id != moved.id, t.parentId == moved.parentId {
                    after = t
                    break
                }
                i -= 1
            }
            var before: TaskItem?
            var j = row
            while j < count, let t = task(at: j) {
                if t.id != moved.id, t.parentId == moved.parentId {
                    before = t
                    break
                }
                j += 1
            }
            guard after?.id != moved.id, before?.id != moved.id, after != nil || before != nil else { return false }
            DispatchQueue.main.async { [session] in
                session.reorder(moved, after: after, before: before)
            }
            return true
        }

        // MARK: Keys, clicks, menu

        /// Returns true when the key was handled.
        func handle(_ event: NSEvent) -> Bool {
            guard let table else { return false }
            let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            switch event.keyCode {
            case 49 where mods.isEmpty:  // space
                toggleSelected()
                return true
            case 36 where mods.isEmpty:  // return
                beginEdit()
                return true
            case 51 where mods.isEmpty, 117 where mods.isEmpty:  // delete, forward delete
                deleteSelected()
                return true
            case 126 where mods == .option:  // up
                moveSelected(up: true)
                return true
            case 125 where mods == .option:  // down
                moveSelected(up: false)
                return true
            default:
                break
            }
            guard Preferences.vimKeys, mods.isEmpty, let chars = event.charactersIgnoringModifiers else { return false }
            switch chars {
            case "j": step(1)
            case "k": step(-1)
            case "x": toggleSelected()
            default: return false
            }
            _ = table
            return true
        }

        @objc func doubleClicked() {
            guard let table, table.clickedRow >= 0, let task = task(at: table.clickedRow) else { return }
            ui.selectedTaskId = task.id
            ui.editingTaskId = task.id
        }

        private var selectedTask: TaskItem? {
            session.find(ui.selectedTaskId)
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
            guard let table, let task = selectedTask else { return }
            let row = table.selectedRow
            session.delete(task)
            // The refresh already ran; keep the selection on the same row.
            if row >= 0, session.count > 0 {
                ui.selectedTaskId = session.task(at: min(row, session.count - 1))?.id
            }
        }

        private func moveSelected(up: Bool) {
            guard let task = selectedTask else { return }
            session.move(task, up: up)
        }

        private func step(_ delta: Int) {
            guard let table, count > 0 else { return }
            let current = table.selectedRow
            let next = min(max(current + delta, 0), count - 1)
            table.selectRowIndexes([next], byExtendingSelection: false)
            table.scrollRowToVisible(next)
        }

        func menuNeedsUpdate(_ menu: NSMenu) {
            menu.removeAllItems()
            guard let table, table.clickedRow >= 0, let task = task(at: table.clickedRow) else { return }
            let session = session
            let ui = ui
            menu.addItem(ActionItem(task.completedAt == nil ? "Complete" : "Mark Incomplete") {
                session.setCompleted(task, task.completedAt == nil)
            })
            menu.addItem(ActionItem("Rename") {
                ui.selectedTaskId = task.id
                ui.editingTaskId = task.id
            })
            let priority = NSMenuItem(title: "Priority", action: nil, keyEquivalent: "")
            let priorities = NSMenu()
            for p in ["high", "medium", "low", "none"] {
                let item = ActionItem(p.capitalized) { session.setPriority(task, p) }
                item.state = task.priority == p ? .on : .off
                priorities.addItem(item)
            }
            priority.submenu = priorities
            menu.addItem(priority)
            let move = NSMenuItem(title: "Move to List", action: nil, keyEquivalent: "")
            let lists = NSMenu()
            let inbox = ActionItem("Inbox") { session.setList(task, "") }
            inbox.state = task.listId == nil ? .on : .off
            lists.addItem(inbox)
            for list in session.lists {
                let item = ActionItem(list.title) { session.setList(task, list.title) }
                item.state = task.listId == list.id ? .on : .off
                lists.addItem(item)
            }
            move.submenu = lists
            menu.addItem(move)
            menu.addItem(.separator())
            menu.addItem(ActionItem("Delete") { session.delete(task) })
        }
    }
}

/// The table itself; keys go to the coordinator first.
final class TableView: NSTableView {
    weak var coordinator: TaskTable.Coordinator?

    override func keyDown(with event: NSEvent) {
        if coordinator?.handle(event) == true { return }
        super.keyDown(with: event)
    }
}

/// One row. The text (day heading, title, details) is drawn straight into
/// the cell in one pass rather than through text-field subviews, so a
/// screen of rows is a screen of draws; the checkbox and the disclosure
/// are controls, and a text field exists only while the title is being
/// edited.
final class TaskCell: NSTableCellView, NSTextFieldDelegate {
    var onToggle: @MainActor (Bool) -> Void = { _ in }
    var onRename: @MainActor (String) -> Void = { _ in }
    var onCancelEdit: @MainActor () -> Void = {}
    var onCollapse: @MainActor () -> Void = {}

    private let disclosure = NSButton()
    private let checkbox = NSButton(checkboxWithTitle: "", target: nil, action: nil)
    private var editor: NSTextField?
    private var dayLine: CTLine?
    private var titleLine: CTLine?
    private var detailsLine: CTLine?
    private var lineWidth: CGFloat = 0
    private var depth = 0
    private var hasDayColumn = false
    private var originalTitle = ""
    /// What the cell shows, so an unchanged row is not drawn again.
    private struct Shown: Equatable {
        var task: TaskItem
        var dayColumn: Bool
        var dayHeading: String?
        var isEditing: Bool
        var isCollapsed: Bool
    }
    private var shown: Shown?
    /// A row in a list, or a card on the board: a card draws its own
    /// surface inset from the row and an accent border when selected.
    enum Style {
        case row
        case card
    }
    let style: Style
    var isHighlighted = false {
        didSet { if isHighlighted != oldValue { needsDisplay = true } }
    }
    /// The row height a card needs: a row plus the gap between cards.
    static let cardRowHeight = Tokens.Size.row + Tokens.Space.xs

    private static let expanded = NSImage(systemSymbolName: "chevron.down", accessibilityDescription: "Collapse subtasks")
    private static let collapsedImage = NSImage(systemSymbolName: "chevron.right", accessibilityDescription: "Expand subtasks")

    init(identifier: NSUserInterfaceItemIdentifier, style: Style = .row) {
        self.style = style
        super.init(frame: .zero)
        self.identifier = identifier
        disclosure.isBordered = false
        disclosure.imagePosition = .imageOnly
        disclosure.target = self
        disclosure.action = #selector(collapseClicked)
        checkbox.target = self
        checkbox.action = #selector(checkboxClicked)
        addSubview(disclosure)
        addSubview(checkbox)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    override var isFlipped: Bool { true }

    func showEmpty(dayColumn: Bool) {
        guard shown != nil || dayColumn != hasDayColumn else { return }
        shown = nil
        hasDayColumn = dayColumn
        disclosure.isHidden = true
        checkbox.isHidden = true
        dayLine = nil
        titleLine = nil
        detailsLine = nil
        endEditing()
        needsLayout = true
        needsDisplay = true
    }

    func show(_ task: TaskItem, dayColumn: Bool, dayHeading: String?, isEditing: Bool, isCollapsed: Bool) {
        let next = Shown(task: task, dayColumn: dayColumn, dayHeading: dayHeading, isEditing: isEditing, isCollapsed: isCollapsed)
        guard next != shown else { return }
        shown = next
        hasDayColumn = dayColumn
        depth = Int(task.depth)
        dayLine = dayHeading.map {
            CTLineCreateWithAttributedString(
                NSAttributedString(
                    string: $0,
                    attributes: [.font: Tokens.Typography.NS.caption, .foregroundColor: Tokens.Colors.NS.textSecondary]))
        }
        disclosure.isHidden = !task.hasSubtasks
        disclosure.image = isCollapsed ? Self.collapsedImage : Self.expanded
        disclosure.contentTintColor = Tokens.Colors.NS.textTertiary
        checkbox.isHidden = false
        checkbox.state = task.completedAt == nil ? .off : .on
        checkbox.setAccessibilityLabel("Complete \(task.title)")
        let done = task.completedAt != nil
        titleLine = CTLineCreateWithAttributedString(
            NSAttributedString(
                string: task.title.isEmpty ? "Untitled" : task.title,
                attributes: [
                    .font: Tokens.Typography.NS.body,
                    .foregroundColor: done ? Tokens.Colors.NS.textTertiary : Tokens.Colors.NS.text,
                    .strikethroughStyle: done ? NSUnderlineStyle.single.rawValue : 0,
                ]))
        let details = Self.details(for: task)
        detailsLine = details.length > 0 ? CTLineCreateWithAttributedString(details) : nil
        lineWidth = 0
        if isEditing, editor == nil {
            beginEditing(task.title)
        } else if !isEditing, editor != nil {
            endEditing()
        }
        needsLayout = true
        needsDisplay = true
    }

    /// The details line is text only: glyphs, not symbol images, so a row
    /// draws with one pass of the typesetter.
    private static func details(for task: TaskItem) -> NSAttributedString {
        let out = NSMutableAttributedString()
        let font = Tokens.Typography.NS.caption
        func append(_ text: String, _ color: NSColor) {
            if out.length > 0 {
                out.append(NSAttributedString(string: "   ", attributes: [.font: font]))
            }
            out.append(NSAttributedString(string: text, attributes: [.font: font, .foregroundColor: color]))
        }
        if let due = DueDisplay.text(for: task) {
            if DueDisplay.isOverdue(task) {
                append("\u{26A0}\u{FE0E} \(due)", Tokens.Colors.NS.overdue)
            } else {
                append(due, Tokens.Colors.NS.textSecondary)
            }
        }
        if let list = task.listTitle {
            append("/\(list)", Tokens.Colors.NS.spanList)
        }
        for tag in task.tags {
            append("#\(tag)", Tokens.Colors.NS.spanTag)
        }
        if task.priority != "none" {
            append("!\(task.priority)", Tokens.Colors.NS.priority(task.priority))
        }
        if task.recurrence != nil {
            append("\u{21BB}", Tokens.Colors.NS.spanRecurrence)
        }
        return out
    }

    // MARK: Geometry (flipped: y grows downward)

    private var titleHeight: CGFloat {
        let f = Tokens.Typography.NS.body
        return f.ascender - f.descender + f.leading
    }

    private var detailsHeight: CGFloat {
        let f = Tokens.Typography.NS.caption
        return f.ascender - f.descender + f.leading
    }

    /// Where the content goes: the whole row, or the card inside it.
    private var contentRect: NSRect {
        switch style {
        case .row: bounds
        case .card: bounds.insetBy(dx: Tokens.Space.xs, dy: Tokens.Space.xxs)
        }
    }

    private var textX: CGFloat {
        var x = contentRect.minX + Tokens.Space.sm
        if hasDayColumn { x += Tokens.Size.dayColumn + Tokens.Space.sm }
        x += CGFloat(depth) * Tokens.Space.xl
        return x + Tokens.Space.lg + Tokens.Space.xs + Tokens.Space.lg + Tokens.Space.sm
    }

    private var titleFrame: NSRect {
        let content = contentRect
        let width = max(content.maxX - textX - Tokens.Space.sm, 0)
        let top = content.minY + (content.height - titleHeight - Tokens.Space.xxs - detailsHeight) / 2
        return NSRect(x: textX, y: top, width: width, height: titleHeight)
    }

    override func layout() {
        super.layout()
        let content = contentRect
        var x = content.minX + Tokens.Space.sm
        if hasDayColumn { x += Tokens.Size.dayColumn + Tokens.Space.sm }
        x += CGFloat(depth) * Tokens.Space.xl
        let control = Tokens.Space.lg
        let y = content.minY + (content.height - control) / 2
        disclosure.frame = NSRect(x: x, y: y, width: control, height: control)
        x += control + Tokens.Space.xs
        checkbox.frame = NSRect(x: x, y: y, width: control, height: control)
        editor?.frame = titleFrame
    }

    /// Text is drawn as CoreText lines laid out once per content change,
    /// truncated to the current width, so a redraw is a glyph run and not
    /// a typesetting pass.
    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        if style == .card, shown != nil {
            let card = NSBezierPath(roundedRect: contentRect.insetBy(dx: 0.5, dy: 0.5), xRadius: Tokens.Radius.md, yRadius: Tokens.Radius.md)
            Tokens.Colors.NS.backgroundSecondary.setFill()
            card.fill()
            (isHighlighted ? Tokens.Colors.NS.accent : Tokens.Colors.NS.separator).setStroke()
            card.lineWidth = isHighlighted ? 2 : 1
            card.stroke()
        }
        let title = titleFrame
        if hasDayColumn, let dayLine {
            Self.draw(dayLine, in: context, x: contentRect.minX + Tokens.Space.sm, baselineY: title.minY, width: Tokens.Size.dayColumn, font: Tokens.Typography.NS.caption)
        }
        if editor == nil, let titleLine {
            Self.draw(titleLine, in: context, x: title.minX, baselineY: title.minY, width: title.width, font: Tokens.Typography.NS.body)
        }
        if let detailsLine {
            Self.draw(detailsLine, in: context, x: title.minX, baselineY: title.maxY + Tokens.Space.xxs, width: title.width, font: Tokens.Typography.NS.caption)
        }
    }

    private static func draw(_ line: CTLine, in context: CGContext, x: CGFloat, baselineY top: CGFloat, width: CGFloat, font: NSFont) {
        var line = line
        if CTLineGetTypographicBounds(line, nil, nil, nil) > width,
            let truncated = CTLineCreateTruncatedLine(line, width, .end, nil)
        {
            line = truncated
        }
        context.saveGState()
        // The cell is flipped; CoreText draws in an unflipped space.
        context.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        context.textPosition = CGPoint(x: x, y: top + font.ascender)
        CTLineDraw(line, context)
        context.restoreGState()
    }

    // MARK: Editing

    private func beginEditing(_ text: String) {
        originalTitle = text
        let field = NSTextField(string: text)
        field.isBordered = false
        field.drawsBackground = false
        field.font = Tokens.Typography.NS.body
        field.textColor = Tokens.Colors.NS.text
        field.lineBreakMode = .byTruncatingTail
        field.cell?.usesSingleLineMode = true
        field.delegate = self
        field.frame = titleFrame
        addSubview(field)
        editor = field
        window?.makeFirstResponder(field)
        field.currentEditor()?.selectAll(nil)
        needsDisplay = true
    }

    private func endEditing() {
        guard let field = editor else { return }
        editor = nil
        if field.currentEditor() != nil {
            window?.makeFirstResponder(superview)
        }
        field.removeFromSuperview()
        needsDisplay = true
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        guard let field = editor else { return }
        let value = field.stringValue
        endEditing()
        if value.trimmingCharacters(in: .whitespacesAndNewlines) != originalTitle {
            onRename(value)
        } else {
            onCancelEdit()
        }
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        if selector == #selector(NSResponder.cancelOperation(_:)) {
            endEditing()
            onCancelEdit()
            return true
        }
        return false
    }

    @objc private func checkboxClicked() {
        onToggle(checkbox.state == .on)
    }

    @objc private func collapseClicked() {
        onCollapse()
    }
}

/// A menu item that runs a closure.
final class ActionItem: NSMenuItem {
    private let handler: @MainActor () -> Void

    init(_ title: String, _ handler: @escaping @MainActor () -> Void) {
        self.handler = handler
        super.init(title: title, action: #selector(run), keyEquivalent: "")
        target = self
    }

    @available(*, unavailable)
    required init(coder: NSCoder) {
        fatalError("not used")
    }

    @objc private func run() {
        let handler = handler
        MainActor.assumeIsolated { handler() }
    }
}
