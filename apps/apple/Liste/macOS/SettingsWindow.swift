// Settings: General, Appearance, Advanced. View preferences live in
// UserDefaults; launch at login in SMAppService; nothing here touches data.

import AppKit
import Carbon.HIToolbox
import ListeKit
import SwiftUI

@MainActor
enum SettingsWindow {
    private static var window: NSWindow?

    static func show(session: Session, delegate: AppDelegate) {
        if window == nil {
            let w = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 520, height: 380),
                styleMask: [.titled, .closable],
                backing: .buffered,
                defer: false)
            w.title = "Settings"
            w.contentView = NSHostingView(rootView: SettingsView(session: session, delegate: delegate))
            w.isReleasedWhenClosed = false
            w.center()
            window = w
        }
        NSApp.setActivationPolicy(.regular)
        window?.makeKeyAndOrderFront(nil)
        NSApp.activate()
    }
}

struct SettingsView: View {
    @Bindable var session: Session
    let delegate: AppDelegate

    var body: some View {
        TabView {
            GeneralSettings(session: session, delegate: delegate)
                .tabItem { Label("General", systemImage: "gearshape") }
            AppearanceSettings()
                .tabItem { Label("Appearance", systemImage: "paintbrush") }
            AdvancedSettings(session: session)
                .tabItem { Label("Advanced", systemImage: "wrench.and.screwdriver") }
        }
        .padding(Tokens.Space.xl)
        .frame(width: 520)
    }
}

struct GeneralSettings: View {
    @Bindable var session: Session
    let delegate: AppDelegate
    @State private var launchAtLogin = LoginItem.isEnabled
    @State private var defaultList = Preferences.defaultList
    @State private var hotKey = Preferences.hotKey
    @State private var recording = false

    var body: some View {
        Form {
            Toggle("Launch at login", isOn: $launchAtLogin)
                .onChange(of: launchAtLogin) { _, on in LoginItem.setEnabled(on) }
            LabeledContent("Quick capture") {
                HStack {
                    Text(HotKeyRecorder.describe(hotKey))
                        .font(Tokens.Typography.body.monospacedDigit())
                        .frame(minWidth: 120, alignment: .leading)
                    Button(recording ? "Press keys\u{2026}" : "Record") { recording = true }
                        .background(HotKeyRecorder(active: $recording) { code, mods in
                            hotKey = (code, mods)
                            Preferences.hotKey = hotKey
                            delegate.reloadHotKey()
                        })
                    Button("Reset") {
                        hotKey = (UInt32(kVK_Space), UInt32(optionKey))
                        Preferences.hotKey = hotKey
                        delegate.reloadHotKey()
                    }
                }
            }
            Picker("Default list", selection: $defaultList) {
                Text("Inbox").tag("")
                ForEach(session.lists, id: \.id) { list in
                    Text(list.title).tag(list.title)
                }
            }
            .onChange(of: defaultList) { _, v in Preferences.defaultList = v }
        }
        .formStyle(.grouped)
    }
}

struct AppearanceSettings: View {
    @State private var theme = Preferences.theme
    @State private var vim = Preferences.vimKeys

    var body: some View {
        Form {
            Picker("Theme", selection: $theme) {
                Text("System").tag("system")
                Text("Light").tag("light")
                Text("Dark").tag("dark")
            }
            .onChange(of: theme) { _, v in
                Preferences.theme = v
                NSApp.appearance = switch v {
                case "light": NSAppearance(named: .aqua)
                case "dark": NSAppearance(named: .darkAqua)
                default: nil
                }
            }
            Toggle("Vim keys (j, k, x)", isOn: $vim)
                .onChange(of: vim) { _, v in Preferences.vimKeys = v }
            Text("The accent color follows the system setting.")
                .font(Tokens.Typography.footnote)
                .foregroundStyle(Tokens.Colors.textSecondary)
        }
        .formStyle(.grouped)
    }
}

struct AdvancedSettings: View {
    @Bindable var session: Session
    @State private var message: String?

    var body: some View {
        Form {
            LabeledContent("Data directory", value: session.status?.dataDir ?? "")
            LabeledContent("Socket", value: session.status?.socketPath ?? "")
            LabeledContent("Command line tools") {
                VStack(alignment: .leading, spacing: Tokens.Space.xs) {
                    Button("Install\u{2026}") { install() }
                    Text("Links liste and liste-mcp from this app into /usr/local/bin.")
                        .font(Tokens.Typography.footnote)
                        .foregroundStyle(Tokens.Colors.textSecondary)
                    if let message {
                        Text(message).font(Tokens.Typography.footnote)
                    }
                }
            }
        }
        .formStyle(.grouped)
    }

    /// Symlink the bundled binaries onto PATH with an administrator prompt.
    private func install() {
        let helpers = Bundle.main.bundleURL.appendingPathComponent("Contents/Helpers")
        let tools = ["liste", "liste-mcp"].filter { FileManager.default.isExecutableFile(atPath: helpers.appendingPathComponent($0).path) }
        guard !tools.isEmpty else {
            message = "This build does not bundle the command line tools."
            return
        }
        let commands = tools.map { "ln -sf '\(helpers.appendingPathComponent($0).path)' '/usr/local/bin/\($0)'" }
        let script = "mkdir -p /usr/local/bin && " + commands.joined(separator: " && ")
        let apple = "do shell script \"\(script.replacingOccurrences(of: "\"", with: "\\\""))\" with administrator privileges"
        var error: NSDictionary?
        NSAppleScript(source: apple)?.executeAndReturnError(&error)
        message = error == nil ? "Installed: \(tools.joined(separator: ", ")) in /usr/local/bin" : "Not installed: \(error?["NSAppleScriptErrorBriefMessage"] ?? "cancelled")"
    }
}

/// Captures the next key combination pressed while active.
struct HotKeyRecorder: NSViewRepresentable {
    @Binding var active: Bool
    var onRecord: @MainActor (UInt32, UInt32) -> Void

    static func describe(_ hotKey: (keyCode: UInt32, modifiers: UInt32)) -> String {
        var parts: [String] = []
        if hotKey.modifiers & UInt32(controlKey) != 0 { parts.append("\u{2303}") }
        if hotKey.modifiers & UInt32(optionKey) != 0 { parts.append("\u{2325}") }
        if hotKey.modifiers & UInt32(shiftKey) != 0 { parts.append("\u{21E7}") }
        if hotKey.modifiers & UInt32(cmdKey) != 0 { parts.append("\u{2318}") }
        parts.append(keyName(hotKey.keyCode))
        return parts.joined()
    }

    static func keyName(_ code: UInt32) -> String {
        switch Int(code) {
        case kVK_Space: return "Space"
        case kVK_Return: return "Return"
        case kVK_Tab: return "Tab"
        case kVK_Escape: return "Esc"
        default: break
        }
        let source = TISCopyCurrentKeyboardLayoutInputSource().takeRetainedValue()
        guard let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) else { return "Key \(code)" }
        let layout = unsafeBitCast(CFDataGetBytePtr(unsafeBitCast(data, to: CFData.self)), to: UnsafePointer<UCKeyboardLayout>.self)
        var dead: UInt32 = 0
        var chars = [UniChar](repeating: 0, count: 4)
        var length = 0
        UCKeyTranslate(layout, UInt16(code), UInt16(kUCKeyActionDisplay), 0, UInt32(LMGetKbdType()), OptionBits(kUCKeyTranslateNoDeadKeysBit), &dead, 4, &length, &chars)
        return String(utf16CodeUnits: chars, count: length).uppercased()
    }

    func makeNSView(context: Context) -> RecorderView {
        let v = RecorderView()
        v.onRecord = { code, mods in
            onRecord(code, mods)
            active = false
        }
        return v
    }

    func updateNSView(_ view: RecorderView, context: Context) {
        if active { view.window?.makeFirstResponder(view) }
    }

    final class RecorderView: NSView {
        var onRecord: ((UInt32, UInt32) -> Void)?
        override var acceptsFirstResponder: Bool { true }

        override func keyDown(with event: NSEvent) {
            var mods: UInt32 = 0
            if event.modifierFlags.contains(.command) { mods |= UInt32(cmdKey) }
            if event.modifierFlags.contains(.option) { mods |= UInt32(optionKey) }
            if event.modifierFlags.contains(.control) { mods |= UInt32(controlKey) }
            if event.modifierFlags.contains(.shift) { mods |= UInt32(shiftKey) }
            guard mods != 0 || event.keyCode == UInt16(kVK_F1) else { return }
            onRecord?(UInt32(event.keyCode), mods)
        }
    }
}
