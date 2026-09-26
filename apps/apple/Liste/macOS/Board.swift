// The board's AppKit half: a horizontal scroller of columns, each one a
// table of cards with rows of one fixed height, so every column is a
// window of its own query like the list is. Cards drag between columns
// and reorder within one through the tables' own drag and drop, which
// starts on the first pixel of movement and shows the system's drop
// marks; columns reorder by dragging their heading. Nothing here decides
// what a drop means: a drop calls back into the SwiftUI side, which asks
// the core to change the one field the column stands for.

import AppKit
import ListeCore
import ListeKit
import SwiftUI

/// One column's window: the count and the pages of cards loaded so far.
@MainActor
final class BoardColumn {
    let key: String
    let title: String
    let query: QuerySpec
    private(set) var count = 0
    private var pages: [Int: [TaskItem]] = [:]
    private let pageSize = 50

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

    func index(of id: String) -> Int? {
        for (page, rows) in pages {
            if let i = rows.firstIndex(where: { $0.id == id }) {
                return page * pageSize + i
            }
        }
        return nil
    }

    /// Load the pages holding `rows` that are not loaded yet.
    func ensure(_ rows: Range<Int>, from session: Session) {
        guard !rows.isEmpty else { return }
        for page in (rows.lowerBound / pageSize)...((rows.upperBound - 1) / pageSize) where pages[page] == nil {
            pages[page] = session.fetch(query, offset: page * pageSize, limit: pageSize)
        }
    }

    func contains(_ id: String) -> Bool {
        index(of: id) != nil
    }
}

/// What a drop or a key asks the board to do.
@MainActor
struct BoardActions {
    /// Put `task` in the column with `key`.
    var assign: (TaskItem, String) -> Void
    /// Move `task` between two neighbours in manual order.
    var reorder: (TaskItem, TaskItem?, TaskItem?) -> Void
    /// Columns in a new order, by key.
    var reorderColumns: ([String]) -> Void
    var select: (String?) -> Void
    var toggleCompleted: (TaskItem) -> Void
}

struct Board: NSViewRepresentable {
    let session: Session
    let ui: UIState
    let columns: [BoardColumn]
    let actions: BoardActions

    func makeNSView(context: Context) -> NSScrollView {
        let container = BoardContainerView()
        container.coordinator = context.coordinator
        let scroll = NSScrollView()
        scroll.documentView = container
        scroll.hasHorizontalScroller = true
        scroll.hasVerticalScroller = false
        scroll.drawsBackground = false
        scroll.horizontalScrollElasticity = .automatic
        context.coordinator.container = container
        context.coordinator.scroll = scroll
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.session = session
        context.coordinator.actions = actions
        context.coordinator.apply(columns: columns, selected: ui.selectedTaskId, generation: session.generation)
    }

    func makeCoordinator() -> Coordinator {
        Coordinator(session: session, actions: actions)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSScrollView, context: Context) -> CGSize? {
        proposal.replacingUnspecifiedDimensions(by: CGSize(width: 640, height: 320))
    }

    @MainActor
    final class Coordinator {
        var session: Session
        var actions: BoardActions
        weak var container: BoardContainerView?
        weak var scroll: NSScrollView?
        private(set) var views: [ColumnView] = []
        private var generation = -1
        private var selected: String?

        init(session: Session, actions: BoardActions) {
            self.session = session
            self.actions = actions
        }

        func apply(columns: [BoardColumn], selected: String?, generation: Int) {
            guard let container else { return }
            // Fresh columns take their rows and selection now, whatever the
            // generation says; existing ones only when the store moved.
            var fresh: [ColumnView] = []
            if views.map(\.column.key) != columns.map(\.key) {
                for view in views { view.removeFromSuperview() }
                views = columns.map { column in
                    let view = ColumnView(column: column, coordinator: self)
                    container.addSubview(view)
                    return view
                }
                fresh = views
                container.needsLayout = true
                container.layoutSubtreeIfNeeded()
            } else {
                for (view, column) in zip(views, columns) { view.column = column }
            }
            let moved = generation != self.generation
            self.generation = generation
            for view in views where moved || fresh.contains(where: { $0 === view }) {
                view.rowsChanged()
            }
            let reselect = selected != self.selected
            self.selected = selected
            for view in views where reselect || fresh.contains(where: { $0 === view }) {
                view.syncSelection(to: selected)
            }
        }

        var columnKeys: [String] { views.map(\.column.key) }

        /// The column, other than `except`, that holds the task with `id`.
        func column(holding id: String) -> ColumnView? {
            views.first { $0.column.contains(id) }
        }
    }
}

/// The document view: columns side by side, and the drop target for a
/// column being dragged to a new place.
final class BoardContainerView: NSView {
    static let columnType = NSPasteboard.PasteboardType("com.example.liste.board-column")
    static let columnWidth: CGFloat = 260
    weak var coordinator: Board.Coordinator?
    private let insertionMark = NSView()
    private var insertionIndex: Int?

    override init(frame: NSRect) {
        super.init(frame: frame)
        registerForDraggedTypes([Self.columnType])
        insertionMark.wantsLayer = true
        insertionMark.layer?.backgroundColor = Tokens.Colors.NS.accent.cgColor
        insertionMark.layer?.cornerRadius = 1
        insertionMark.isHidden = true
        addSubview(insertionMark)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    override var isFlipped: Bool { true }

    override func layout() {
        super.layout()
        guard let coordinator else { return }
        let gap = Tokens.Space.md
        let height = max(superview?.bounds.height ?? bounds.height, 0)
        var x = gap
        for view in coordinator.views {
            view.frame = NSRect(x: x, y: Tokens.Space.sm, width: Self.columnWidth, height: max(height - 2 * Tokens.Space.sm, 0))
            x += Self.columnWidth + gap
        }
        let width = max(x, superview?.bounds.width ?? 0)
        if frame.size != NSSize(width: width, height: height) {
            setFrameSize(NSSize(width: width, height: height))
        }
        if let index = insertionIndex {
            let columns = coordinator.views
            let markX = index < columns.count ? columns[index].frame.minX - gap / 2 : (columns.last.map { $0.frame.maxX + gap / 2 } ?? gap)
            insertionMark.frame = NSRect(x: markX - 1, y: Tokens.Space.sm, width: 2, height: max(height - 2 * Tokens.Space.sm, 0))
        }
    }

    override func viewDidMoveToSuperview() {
        super.viewDidMoveToSuperview()
        superview?.postsFrameChangedNotifications = true
        if let superview {
            NotificationCenter.default.addObserver(
                self, selector: #selector(superviewResized), name: NSView.frameDidChangeNotification, object: superview)
        }
    }

    @objc private func superviewResized() {
        needsLayout = true
    }

    // MARK: Column drops

    private func index(for info: NSDraggingInfo) -> Int {
        guard let coordinator else { return 0 }
        let x = convert(info.draggingLocation, from: nil).x
        var index = 0
        for view in coordinator.views where x > view.frame.midX {
            index += 1
        }
        return index
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        draggingUpdated(sender)
    }

    override func draggingUpdated(_ sender: NSDraggingInfo) -> NSDragOperation {
        guard sender.draggingPasteboard.availableType(from: [Self.columnType]) != nil else { return [] }
        insertionIndex = index(for: sender)
        insertionMark.isHidden = false
        needsLayout = true
        return .move
    }

    override func draggingExited(_ sender: NSDraggingInfo?) {
        insertionIndex = nil
        insertionMark.isHidden = true
    }

    override func draggingEnded(_ sender: NSDraggingInfo) {
        insertionIndex = nil
        insertionMark.isHidden = true
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        insertionIndex = nil
        insertionMark.isHidden = true
        guard let coordinator, let key = sender.draggingPasteboard.string(forType: Self.columnType) else { return false }
        var keys = coordinator.columnKeys
        guard let from = keys.firstIndex(of: key) else { return false }
        var to = index(for: sender)
        keys.remove(at: from)
        if to > from { to -= 1 }
        keys.insert(key, at: min(to, keys.count))
        if keys != coordinator.columnKeys {
            coordinator.actions.reorderColumns(keys)
        }
        return true
    }
}

/// A heading and a table of cards.
final class ColumnView: NSView, NSTableViewDataSource, NSTableViewDelegate, NSDraggingSource {
    private static let cellId = NSUserInterfaceItemIdentifier("board-card")
    var column: BoardColumn {
        didSet { header.set(title: column.title, count: column.count) }
    }
    private unowned let coordinator: Board.Coordinator
    private let header = ColumnHeaderView()
    private let table = BoardTableView()
    private let scroll = NSScrollView()
    private var count = 0
    private var syncingSelection = false
    private var loadRequested = false
    private var dragStart: NSPoint?

    init(column: BoardColumn, coordinator: Board.Coordinator) {
        self.column = column
        self.coordinator = coordinator
        super.init(frame: .zero)
        header.set(title: column.title, count: column.count)
        header.onDrag = { [weak self] event in self?.beginColumnDrag(event) }
        addSubview(header)
        let tableColumn = NSTableColumn(identifier: .init("card"))
        tableColumn.resizingMask = .autoresizingMask
        table.addTableColumn(tableColumn)
        table.headerView = nil
        table.rowHeight = TaskCell.cardRowHeight
        table.usesAutomaticRowHeights = false
        table.style = .plain
        table.selectionHighlightStyle = .none
        table.backgroundColor = .clear
        table.allowsMultipleSelection = false
        table.allowsEmptySelection = true
        table.allowsTypeSelect = false
        table.intercellSpacing = .zero
        table.columnAutoresizingStyle = .firstColumnOnlyAutoresizingStyle
        table.dataSource = self
        table.delegate = self
        table.column = self
        table.registerForDraggedTypes([TaskTable.Coordinator.dragType])
        table.setDraggingSourceOperationMask(.move, forLocal: true)
        table.draggingDestinationFeedbackStyle = .gap
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(
            self, selector: #selector(scrolled), name: NSView.boundsDidChangeNotification, object: scroll.contentView)
        addSubview(scroll)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    override var isFlipped: Bool { true }

    override func layout() {
        super.layout()
        let headerHeight = Tokens.Space.xl
        header.frame = NSRect(x: 0, y: 0, width: bounds.width, height: headerHeight)
        scroll.frame = NSRect(x: 0, y: headerHeight + Tokens.Space.xs, width: bounds.width, height: max(bounds.height - headerHeight - Tokens.Space.xs, 0))
    }

    // MARK: Rows

    private var visibleRows: Range<Int> {
        let rows = table.rows(in: table.visibleRect)
        return rows.location..<(rows.location + rows.length)
    }

    /// The column re-counted: note the count, refill what is on screen.
    func rowsChanged() {
        header.set(title: column.title, count: column.count)
        if count != column.count {
            count = column.count
            table.noteNumberOfRowsChanged()
        }
        column.ensure(visibleRows, from: coordinator.session)
        reconfigureVisibleRows()
    }

    @objc private func scrolled() {
        let rows = visibleRows
        guard !rows.isEmpty else { return }
        column.ensure(rows, from: coordinator.session)
    }

    private func requestLoad() {
        guard !loadRequested else { return }
        loadRequested = true
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            loadRequested = false
            column.ensure(visibleRows, from: coordinator.session)
            reconfigureVisibleRows()
        }
    }

    private func reconfigureVisibleRows() {
        for row in visibleRows {
            if let cell = table.view(atColumn: 0, row: row, makeIfNecessary: false) as? TaskCell {
                configure(cell, row: row)
            }
        }
    }

    func syncSelection(to id: String?) {
        let row = id.flatMap(column.index(of:))
        syncingSelection = true
        if let row {
            table.selectRowIndexes([row], byExtendingSelection: false)
        } else {
            table.deselectAll(nil)
        }
        syncingSelection = false
        reconfigureVisibleRows()
    }

    func numberOfRows(in tableView: NSTableView) -> Int {
        count
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        let cell = (tableView.makeView(withIdentifier: Self.cellId, owner: nil) as? TaskCell) ?? TaskCell(identifier: Self.cellId, style: .card)
        configure(cell, row: row)
        return cell
    }

    private func configure(_ cell: TaskCell, row: Int) {
        guard let task = column.task(at: row) else {
            cell.showEmpty(dayColumn: false)
            requestLoad()
            return
        }
        cell.show(task, dayColumn: false, dayHeading: nil, isEditing: false, isCollapsed: false)
        cell.isHighlighted = table.selectedRow == row
        let actions = coordinator.actions
        cell.onToggle = { _ in actions.toggleCompleted(task) }
    }

    func tableViewSelectionDidChange(_ notification: Notification) {
        guard !syncingSelection else { return }
        let id = table.selectedRow >= 0 ? column.task(at: table.selectedRow)?.id : nil
        if let id {
            coordinator.actions.select(id)
        }
        reconfigureVisibleRows()
    }

    // MARK: Card drag and drop

    func tableView(_ tableView: NSTableView, pasteboardWriterForRow row: Int) -> NSPasteboardWriting? {
        guard let task = column.task(at: row) else { return nil }
        let item = NSPasteboardItem()
        item.setString(task.id, forType: TaskTable.Coordinator.dragType)
        return item
    }

    func tableView(
        _ tableView: NSTableView, validateDrop info: NSDraggingInfo, proposedRow row: Int,
        proposedDropOperation dropOperation: NSTableView.DropOperation
    ) -> NSDragOperation {
        guard info.draggingPasteboard.availableType(from: [TaskTable.Coordinator.dragType]) != nil else { return [] }
        if info.draggingSource as? NSTableView === tableView {
            // Within the column: a place between two cards.
            if dropOperation == .on {
                tableView.setDropRow(row, dropOperation: .above)
            }
        } else {
            // From elsewhere: the whole column.
            tableView.setDropRow(-1, dropOperation: .on)
        }
        return .move
    }

    func tableView(
        _ tableView: NSTableView, acceptDrop info: NSDraggingInfo, row: Int,
        dropOperation: NSTableView.DropOperation
    ) -> Bool {
        guard let id = info.draggingPasteboard.string(forType: TaskTable.Coordinator.dragType) else { return false }
        let actions = coordinator.actions
        if info.draggingSource as? NSTableView === tableView {
            guard let from = column.index(of: id), let moved = column.task(at: from) else { return false }
            var after: TaskItem?
            var i = row - 1
            while i >= 0, let t = column.task(at: i) {
                if t.id != moved.id, t.parentId == moved.parentId {
                    after = t
                    break
                }
                i -= 1
            }
            var before: TaskItem?
            var j = row
            while j < count, let t = column.task(at: j) {
                if t.id != moved.id, t.parentId == moved.parentId {
                    before = t
                    break
                }
                j += 1
            }
            guard after != nil || before != nil else { return false }
            DispatchQueue.main.async { actions.reorder(moved, after, before) }
            return true
        }
        guard let source = coordinator.column(holding: id), let task = source.column.task(at: source.column.index(of: id) ?? -1) else { return false }
        let key = column.key
        DispatchQueue.main.async { actions.assign(task, key) }
        return true
    }

    // MARK: Column drag

    private func beginColumnDrag(_ event: NSEvent) {
        let item = NSPasteboardItem()
        item.setString(column.key, forType: BoardContainerView.columnType)
        let dragging = NSDraggingItem(pasteboardWriter: item)
        let image = snapshot()
        dragging.setDraggingFrame(bounds, contents: image)
        let session = beginDraggingSession(with: [dragging], event: event, source: self)
        session.animatesToStartingPositionsOnCancelOrFail = true
    }

    private func snapshot() -> NSImage? {
        guard let rep = bitmapImageRepForCachingDisplay(in: bounds) else { return nil }
        cacheDisplay(in: bounds, to: rep)
        let image = NSImage(size: bounds.size)
        image.addRepresentation(rep)
        return image
    }

    func draggingSession(_ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext) -> NSDragOperation {
        context == .withinApplication ? .move : []
    }

    // MARK: Keys

    /// Returns true when the key was handled.
    func handle(_ event: NSEvent) -> Bool {
        let mods = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        guard table.selectedRow >= 0, let task = column.task(at: table.selectedRow) else { return false }
        let keys = coordinator.columnKeys
        let here = keys.firstIndex(of: column.key) ?? 0
        switch event.keyCode {
        case 49 where mods.isEmpty:  // space
            coordinator.actions.toggleCompleted(task)
            return true
        case 123 where mods == .option:  // left
            guard here > 0 else { return true }
            coordinator.actions.assign(task, keys[here - 1])
            return true
        case 124 where mods == .option:  // right
            guard here + 1 < keys.count else { return true }
            coordinator.actions.assign(task, keys[here + 1])
            return true
        default:
            return false
        }
    }
}

/// The heading: title, count, and the handle for dragging the column.
final class ColumnHeaderView: NSView {
    var onDrag: ((NSEvent) -> Void)?
    private let title = NSTextField(labelWithString: "")
    private let count = NSTextField(labelWithString: "")
    private var pressed: NSPoint?

    override init(frame: NSRect) {
        super.init(frame: frame)
        title.font = Tokens.Typography.NS.bodyStrong
        title.textColor = Tokens.Colors.NS.text
        title.lineBreakMode = .byTruncatingTail
        count.font = Tokens.Typography.NS.caption
        count.textColor = Tokens.Colors.NS.textTertiary
        addSubview(title)
        addSubview(count)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        nil
    }

    override var isFlipped: Bool { true }

    func set(title text: String, count n: Int) {
        title.stringValue = text
        count.stringValue = "\(n)"
        needsLayout = true
    }

    override func layout() {
        super.layout()
        title.sizeToFit()
        count.sizeToFit()
        let y = (bounds.height - title.frame.height) / 2
        title.frame.origin = NSPoint(x: Tokens.Space.sm, y: y)
        title.frame.size.width = min(title.frame.width, bounds.width - 2 * Tokens.Space.sm - count.frame.width - Tokens.Space.sm)
        count.frame.origin = NSPoint(x: title.frame.maxX + Tokens.Space.sm, y: (bounds.height - count.frame.height) / 2)
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .openHand)
    }

    override func mouseDown(with event: NSEvent) {
        pressed = event.locationInWindow
    }

    override func mouseDragged(with event: NSEvent) {
        guard let pressed else { return }
        let moved = hypot(event.locationInWindow.x - pressed.x, event.locationInWindow.y - pressed.y)
        if moved > 4 {
            self.pressed = nil
            onDrag?(event)
        }
    }

    override func mouseUp(with event: NSEvent) {
        pressed = nil
    }
}

/// A column's table; keys go to the column first.
final class BoardTableView: NSTableView {
    weak var column: ColumnView?

    override func keyDown(with event: NSEvent) {
        if column?.handle(event) == true { return }
        super.keyDown(with: event)
    }
}
