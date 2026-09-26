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
    private var mainWindow: NSWindow?
    private var statusItem: NSStatusItem?
    private var capturePanel: QuickCapturePanel?
    private var hotKey: HotKey?
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
        hotKey = HotKey(keyCode: HotKey.space, modifiers: HotKey.option) { [weak self] in
            self?.showQuickCapture()
        }
        // The panel is built now, while nobody is waiting, so the hotkey
        // only has to order it front (Section 4: under 50 ms).
        capturePanel = QuickCapturePanel(session: session)
        if background {
            NSApp.setActivationPolicy(.accessory)
            log.info("started in the background; socket ready at \(self.session.status?.socketPath ?? "?")")
        } else {
            openMainWindow()
        }
        #if DEBUG
            // `--measure-quick-capture`: show the panel once after launch and
            // log how long it took to appear, for the Section 4 budget.
            if CommandLine.arguments.contains("--measure-quick-capture") {
                DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
                    self?.showQuickCapture()
                }
            }
        #endif
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
            window.contentView = NSHostingView(rootView: MainView(session: session))
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
        let appItem = NSMenuItem()
        let appMenu = NSMenu()
        appMenu.addItem(withTitle: "About Liste", action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)), keyEquivalent: "")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Hide Liste", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        appMenu.addItem(.separator())
        appMenu.addItem(withTitle: "Quit Liste", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        appItem.submenu = appMenu
        main.addItem(appItem)

        let fileItem = NSMenuItem()
        let fileMenu = NSMenu(title: "File")
        let quick = fileMenu.addItem(withTitle: "Quick Capture", action: #selector(captureFromMenu), keyEquivalent: "n")
        quick.target = self
        fileMenu.addItem(withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        fileItem.submenu = fileMenu
        main.addItem(fileItem)

        let editItem = NSMenuItem()
        let editMenu = NSMenu(title: "Edit")
        let undo = editMenu.addItem(withTitle: "Undo", action: #selector(undoFromMenu), keyEquivalent: "z")
        undo.target = self
        let redo = editMenu.addItem(withTitle: "Redo", action: #selector(redoFromMenu), keyEquivalent: "Z")
        redo.target = self
        editMenu.addItem(.separator())
        editMenu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        editMenu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        editMenu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        editMenu.addItem(withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
        editItem.submenu = editMenu
        main.addItem(editItem)

        let windowItem = NSMenuItem()
        let windowMenu = NSMenu(title: "Window")
        let show = windowMenu.addItem(withTitle: "Liste", action: #selector(openFromMenu), keyEquivalent: "0")
        show.target = self
        windowMenu.addItem(withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        windowItem.submenu = windowMenu
        main.addItem(windowItem)
        NSApp.mainMenu = main
        NSApp.windowsMenu = windowMenu
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
