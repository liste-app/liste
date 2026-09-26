// The host process (Section 3). This app is the one process on the machine
// that owns the store: it takes the lock, opens the store, and serves the
// IPC socket, all inside the core's host started here. It runs happily
// with no window: launched with `--background` it never shows one and has
// no Dock icon; closing the last window drops to the menu bar; only Quit
// stops the host.

import AppKit
import ListeCore
import ListeKit
import OSLog
import SwiftUI

let log = Logger(subsystem: "com.example.liste", category: "app")

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    private(set) var session: Session!
    let ui = UIState()
    private var mainWindow: NSWindow?
    private var statusItem: NSStatusItem?
    private var capturePanel: QuickCapturePanel?
    private var hotKey: HotKey?
    private var reminders: Reminders?
    private let background = CommandLine.arguments.contains("--background")

    func applicationDidFinishLaunching(_ notification: Notification) {
        buildMainMenu()
        do {
            session = try Session()
        } catch {
            let alert = NSAlert()
            alert.alertStyle = .critical
            alert.messageText = "Liste is already running"
            alert.informativeText = describe(error)
            alert.addButton(withTitle: "Quit")
            NSApp.setActivationPolicy(.regular)
            alert.runModal()
            NSApp.terminate(nil)
            return
        }
        LoginItem.registerOnFirstRun()
        installStatusItem()
        let (code, mods) = Preferences.hotKey
        hotKey = HotKey(keyCode: code, modifiers: mods) { [weak self] in
            self?.showQuickCapture()
        }
        // The panel is built now, while nobody is waiting, so the hotkey
        // only has to order it front (Section 4: under 50 ms).
        capturePanel = QuickCapturePanel(session: session)
        let reminders = Reminders(session: session)
        self.reminders = reminders
        session.onStoreChange = { [weak reminders] in reminders?.reschedule() }
        reminders.reschedule()
        NotificationCenter.default.addObserver(forName: .quickCaptureRequested, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.showQuickCapture() }
        }
        NotificationCenter.default.addObserver(forName: .settingsRequested, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.showSettings() }
        }
        if background {
            NSApp.setActivationPolicy(.accessory)
            log.info("started in the background; socket ready at \(self.session.status?.socketPath ?? "?")")
        } else {
            openMainWindow()
        }
        do {
            // `--measure-quick-capture`: show the panel once after launch and
            // log how long it took to appear, for the Section 4 budget.
            if CommandLine.arguments.contains("--measure-quick-capture") {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
                    self?.showQuickCapture()
                }
            }
            // `--measure-scroll`: fill the store with the 50,000-task fixture
            // if it is smaller, show Anytime, and log the frame rate while
            // the list scrolls. Only meaningful against a scratch store.
            if CommandLine.arguments.contains("--measure-scroll") {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
                    self?.measureScroll()
                }
            }
        }
    }

    private func measureScroll() {
        session.selection = .today
        session.populateFixtureIfSmall(50_000)
        guard let scroll = mainWindow?.contentView?.firstScrollView() else {
            FileHandle.standardError.write(Data("scroll: no scroll view\n".utf8))
            return
        }
        let meter = FrameMeter()
        let steps = 240
        Task { @MainActor in
            // Let the window settle after the fixture load before timing.
            try? await Task.sleep(for: .seconds(2))
            // Opening Anytime from Today: three opens to warm up, then ten
            // measured; the median is the number reported. Main-thread time
            // is the thread's CPU time until the run loop goes idle; wall
            // time is beside it.
            var opens: [MainThreadTimer.Sample] = []
            for i in 1...13 {
                self.session.selection = .today
                try? await Task.sleep(for: .milliseconds(250))
                let sample = await MainThreadTimer.measure { self.session.selection = .anytime }
                if i > 3 { opens.append(sample) }
                try? await Task.sleep(for: .milliseconds(250))
            }
            let sorted = opens.sorted { $0.cpuMilliseconds < $1.cpuMilliseconds }
            let median = sorted[sorted.count / 2]
            let rows = self.session.count
            let query = self.session.lastQueryMilliseconds
            FileHandle.standardError.write(
                Data(String(format: "open: Anytime with %d rows in %.1f ms main thread (%.1f ms wall to idle), median of %d after 3 warm-ups, queries %.1f ms; all: %@\n",
                    rows, median.cpuMilliseconds, median.wallMilliseconds, opens.count, query,
                    opens.map { String(format: "%.1f", $0.cpuMilliseconds) }.joined(separator: " ")).utf8))
            // A refresh in place, as a change notification causes: the same
            // rows re-read and redrawn, nothing else moving.
            var refreshes: [Double] = []
            for _ in 1...10 {
                try? await Task.sleep(for: .milliseconds(250))
                refreshes.append(await MainThreadTimer.measure { self.session.refresh() }.cpuMilliseconds)
            }
            let middle = refreshes.sorted()[refreshes.count / 2]
            FileHandle.standardError.write(
                Data(String(format: "refresh: in place in %.1f ms main thread, median of %d; all: %@\n",
                    middle, refreshes.count, refreshes.map { String(format: "%.1f", $0) }.joined(separator: " ")).utf8))
            try? await Task.sleep(for: .seconds(1))
            guard let scroll = self.mainWindow?.contentView?.firstScrollView() else { return }
            let total = scroll.documentView?.frame.height ?? 0
            meter.start()
            var worst = 0.0
            var sum = 0.0
            for step in 1...steps {
                try? await Task.sleep(for: .milliseconds(16))
                let y = total * CGFloat(step) / CGFloat(steps)
                let sample = await MainThreadTimer.measure {
                    scroll.contentView.scroll(to: NSPoint(x: 0, y: y))
                    scroll.reflectScrolledClipView(scroll.contentView)
                }
                worst = max(worst, sample.cpuMilliseconds)
                sum += sample.cpuMilliseconds
            }
            let report = meter.stop()
            FileHandle.standardError.write(
                Data(String(format: "scroll: %@ over %d rows; main thread per step avg %.1f ms, worst %.1f ms\n",
                    report, self.session.count, sum / Double(steps), worst).utf8))
            log.debug("scroll: \(report)")
        }
        _ = scroll
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows: Bool) -> Bool {
        openMainWindow()
        return true
    }

    func applicationWillTerminate(_ notification: Notification) {
        hotKey?.unregister()
        hotKey = nil
        session?.shutdown()
        log.info("host stopped")
    }

    // MARK: Windows

    func openMainWindow() {
        NSApp.setActivationPolicy(.regular)
        if mainWindow == nil {
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 900, height: 600),
                styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                backing: .buffered,
                defer: false
            )
            window.title = "Liste"
            window.titlebarAppearsTransparent = true
            window.contentView = NSHostingView(rootView: MainView(session: session, ui: ui))
            window.setFrameAutosaveName("Main")
            window.center()
            window.isReleasedWhenClosed = false
            window.delegate = self
            mainWindow = window
        }
        mainWindow?.makeKeyAndOrderFront(nil)
        NSApp.activate()
    }

    func windowWillClose(_ notification: Notification) {
        // Closing the last window drops to the menu bar; the host keeps
        // running for the CLI, the MCP server, and the hotkey.
        DispatchQueue.main.async {
            if NSApp.windows.allSatisfy({ !$0.isVisible || $0 is NSPanel }) {
                NSApp.setActivationPolicy(.accessory)
            }
        }
    }

    func showQuickCapture() {
        let started = CACurrentMediaTime()
        if capturePanel == nil {
            capturePanel = QuickCapturePanel(session: session)
        }
        capturePanel?.present(startedAt: started)
    }

    // MARK: Menu bar item

    private func installStatusItem() {
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = NSImage(systemSymbolName: "checklist", accessibilityDescription: "Liste")
        let menu = NSMenu()
        menu.addItem(withTitle: "Open Liste", action: #selector(openFromMenu), keyEquivalent: "")
        let capture = menu.addItem(withTitle: "Quick Capture", action: #selector(captureFromMenu), keyEquivalent: " ")
        capture.keyEquivalentModifierMask = [.option]
        menu.addItem(.separator())
        let login = menu.addItem(withTitle: "Launch at Login", action: #selector(toggleLoginItem), keyEquivalent: "")
        login.state = LoginItem.isEnabled ? .on : .off
        menu.addItem(.separator())
        menu.addItem(withTitle: "Quit Liste", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        menu.items.forEach { $0.target = $0.action == #selector(NSApplication.terminate(_:)) ? NSApp : self }
        item.menu = menu
        statusItem = item
    }

    @objc private func openFromMenu() {
        openMainWindow()
    }

    @objc private func captureFromMenu() {
        showQuickCapture()
    }

    @objc private func toggleLoginItem(_ sender: NSMenuItem) {
        LoginItem.setEnabled(!LoginItem.isEnabled)
        sender.state = LoginItem.isEnabled ? .on : .off
    }

    // MARK: Main menu

    private func buildMainMenu() {
        let main = NSMenu()
        func add(_ menu: NSMenu, _ title: String, _ action: Selector, _ key: String, _ mods: NSEvent.ModifierFlags = [.command], target: AnyObject? = nil) {
            let item = menu.addItem(withTitle: title, action: action, keyEquivalent: key)
            item.keyEquivalentModifierMask = mods
            item.target = target
        }
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        add(appMenu, "About Liste", #selector(NSApplication.orderFrontStandardAboutPanel(_:)), "", [])
        appMenu.addItem(.separator())
        add(appMenu, "Settings\u{2026}", #selector(showSettings), ",", target: self)
        appMenu.addItem(.separator())
        add(appMenu, "Hide Liste", #selector(NSApplication.hide(_:)), "h")
        add(appMenu, "Quit Liste", #selector(NSApplication.terminate(_:)), "q")
        appItem.submenu = appMenu
        main.addItem(appItem)

        let fileItem = NSMenuItem()
        let fileMenu = NSMenu(title: "File")
        add(fileMenu, "New Task", #selector(newTask), "n", target: self)
        add(fileMenu, "New List\u{2026}", #selector(newList), "n", [.command, .shift], target: self)
        add(fileMenu, "Quick Capture", #selector(captureFromMenu), " ", [.option], target: self)
        fileMenu.addItem(.separator())
        add(fileMenu, "Close", #selector(NSWindow.performClose(_:)), "w")
        fileItem.submenu = fileMenu
        main.addItem(fileItem)

        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        add(editMenu, "Undo", #selector(undoFromMenu), "z", target: self)
        add(editMenu, "Redo", #selector(redoFromMenu), "z", [.command, .shift], target: self)
        editMenu.addItem(.separator())
        add(editMenu, "Cut", #selector(NSText.cut(_:)), "x")
        add(editMenu, "Copy", #selector(NSText.copy(_:)), "c")
        add(editMenu, "Paste", #selector(NSText.paste(_:)), "v")
        add(editMenu, "Select All", #selector(NSText.selectAll(_:)), "a")
        editMenu.addItem(.separator())
        add(editMenu, "Complete", #selector(completeSelected), " ", [], target: self)
        add(editMenu, "Rename", #selector(renameSelected), "\r", [], target: self)
        add(editMenu, "Delete", #selector(deleteSelected), "\u{8}", [], target: self)
        add(editMenu, "Move Up", #selector(moveUp), String(UnicodeScalar(NSUpArrowFunctionKey)!), [.option], target: self)
        add(editMenu, "Move Down", #selector(moveDown), String(UnicodeScalar(NSDownArrowFunctionKey)!), [.option], target: self)
        editItem.submenu = editMenu
        main.addItem(editItem)

        let viewItem = NSMenuItem()
        let viewMenu = NSMenu(title: "View")
        add(viewMenu, "Search", #selector(focusSearch), "f", target: self)
        add(viewMenu, "Command Palette", #selector(showPalette), "k", target: self)
        add(viewMenu, "Show Inspector", #selector(toggleInspector), "i", target: self)
        viewMenu.addItem(.separator())
        add(viewMenu, "List", #selector(showList), "1", [.command, .shift], target: self)
        add(viewMenu, "Board", #selector(showBoard), "2", [.command, .shift], target: self)
        viewItem.submenu = viewMenu
        main.addItem(viewItem)

        let goItem = NSMenuItem()
        let goMenu = NSMenu(title: "Go")
        add(goMenu, "Today", #selector(goToday), "1", target: self)
        add(goMenu, "Upcoming", #selector(goUpcoming), "2", target: self)
        add(goMenu, "Anytime", #selector(goAnytime), "3", target: self)
        add(goMenu, "Completed", #selector(goCompleted), "4", target: self)
        add(goMenu, "Inbox", #selector(goInbox), "5", target: self)
        goItem.submenu = goMenu
        main.addItem(goItem)

        let windowItem = NSMenuItem()
        let windowMenu = NSMenu(title: "Window")
        add(windowMenu, "Liste", #selector(openFromMenu), "0", target: self)
        add(windowMenu, "Minimize", #selector(NSWindow.performMiniaturize(_:)), "m")
        windowItem.submenu = windowMenu
        main.addItem(windowItem)
        NSApp.mainMenu = main
        NSApp.windowsMenu = windowMenu
    }

    @objc private func newTask() { openMainWindow(); ui.newTaskRequest += 1 }
    @objc private func newList() { openMainWindow(); ui.newListRequest += 1 }
    @objc private func focusSearch() { openMainWindow(); ui.searchFocusRequest += 1 }
    @objc private func showPalette() { openMainWindow(); ui.paletteRequest += 1 }
    @objc private func toggleInspector() { ui.showInspector.toggle() }
    @objc private func showList() { ui.viewMode = .list }
    @objc private func showBoard() { ui.viewMode = .board }
    @objc private func goToday() { session.selection = .today }
    @objc private func goUpcoming() { session.selection = .upcoming }
    @objc private func goAnytime() { session.selection = .anytime }
    @objc private func goCompleted() { session.selection = .completed }
    @objc private func goInbox() { session.selection = .inbox }
    @objc private func showSettings() { SettingsWindow.show(session: session, delegate: self) }

    @objc private func completeSelected() {
        guard let task = session.find(ui.selectedTaskId) else { return }
        session.setCompleted(task, task.completedAt == nil)
    }

    @objc private func renameSelected() { ui.editRequest += 1 }

    @objc private func deleteSelected() {
        guard let task = session.find(ui.selectedTaskId) else { return }
        session.delete(task)
    }

    @objc private func moveUp() {
        guard let task = session.find(ui.selectedTaskId) else { return }
        session.move(task, up: true)
    }

    @objc private func moveDown() {
        guard let task = session.find(ui.selectedTaskId) else { return }
        session.move(task, up: false)
    }

    /// Re-register the quick-capture hotkey after Settings changed it.
    func reloadHotKey() {
        hotKey?.unregister()
        let (code, mods) = Preferences.hotKey
        hotKey = HotKey(keyCode: code, modifiers: mods) { [weak self] in
            self?.showQuickCapture()
        }
    }

    @objc private func undoFromMenu() {
        session.undo()
    }

    @objc private func redoFromMenu() {
        session.redo()
    }

    private func describe(_ error: Error) -> String {
        if let e = error as? ListeError {
            return "\(e)\n\nQuit the other copy of Liste, or the `liste daemon`, and open Liste again."
        }
        return error.localizedDescription
    }
}
