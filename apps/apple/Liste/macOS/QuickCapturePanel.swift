// The quick-capture panel (Section 4: appears in under 50 ms). A floating,
// non-activating panel created once and kept around, so showing it is an
// orderFront and a focus change, nothing more. Spans come from the core's
// preview on every keystroke; Return captures, Escape dismisses.

import AppKit
import ListeCore
import ListeKit
import SwiftUI

@MainActor
final class QuickCapturePanel: NSPanel {
    private let session: Session
    private var model: CaptureModel

    init(session: Session) {
        self.session = session
        self.model = CaptureModel(session: session)
        super.init(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 120),
            styleMask: [.nonactivatingPanel, .titled, .fullSizeContentView, .hudWindow],
            backing: .buffered,
            defer: false
        )
        titleVisibility = .hidden
        titlebarAppearsTransparent = true
        isMovableByWindowBackground = true
        level = .floating
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .transient]
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        let model = self.model
        contentView = NSHostingView(rootView: CaptureView(model: model, dismiss: { [weak self] in self?.dismiss() }))
        // Building the view hierarchy once, off-screen, is what keeps the
        // first appearance under budget.
        contentView?.layoutSubtreeIfNeeded()
    }

    override var canBecomeKey: Bool { true }

    func present(startedAt: CFTimeInterval) {
        model.reset()
        if let screen = NSScreen.main {
            let frame = screen.visibleFrame
            let size = self.frame.size
            setFrameOrigin(NSPoint(x: frame.midX - size.width / 2, y: frame.midY + frame.height * 0.15))
        }
        makeKeyAndOrderFront(nil)
        model.focus = true
        if CommandLine.arguments.contains("--measure-quick-capture") {
            DispatchQueue.main.async {
                let ms = (CACurrentMediaTime() - startedAt) * 1000
                log.debug("quick capture panel visible in \(ms, format: .fixed(precision: 1)) ms")
                FileHandle.standardError.write(Data("quick capture: \(String(format: "%.1f", ms)) ms\n".utf8))
            }
        }
    }

    func dismiss() {
        orderOut(nil)
        model.reset()
    }

    override func cancelOperation(_ sender: Any?) {
        dismiss()
    }

    override func resignKey() {
        super.resignKey()
        orderOut(nil)
    }
}

@MainActor
@Observable
final class CaptureModel {
    var text: String = "" {
        didSet { preview = text.isEmpty ? nil : session.preview(text) }
    }
    var preview: CapturePreview?
    var focus: Bool = false
    var error: String?
    private let session: Session

    init(session: Session) {
        self.session = session
    }

    func reset() {
        text = ""
        preview = nil
        error = nil
    }

    /// Commit the line. Returns whether the panel should close.
    func commit() -> Bool {
        let line = text.trimmingCharacters(in: .whitespaces)
        guard !line.isEmpty else { return true }
        do {
            _ = try session.capture(line)
            return true
        } catch {
            self.error = "\(error)"
            return false
        }
    }
}

struct CaptureView: View {
    @Bindable var model: CaptureModel
    var dismiss: () -> Void
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField("Capture a task", text: $model.text)
                .textFieldStyle(.plain)
                .font(.system(size: 22, weight: .regular))
                .focused($focused)
                .onSubmit {
                    if model.commit() { dismiss() }
                }
                .accessibilityLabel("Quick capture")
            Group {
                if let preview = model.preview, !model.text.isEmpty {
                    Text(highlighted(model.text, spans: preview.spans))
                        .font(.system(size: 14))
                        .lineLimit(1)
                    Text(summary(of: preview).isEmpty ? "Return to add \u{201C}\(preview.title)\u{201D}" : summary(of: preview))
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else {
                    Text("Dates, times, #tags, !priority, /list, and \u{201C}every \u{2026}\u{201D} are understood.")
                        .font(.system(size: 12))
                        .foregroundStyle(.tertiary)
                }
            }
            .frame(height: 40, alignment: .topLeading)
            if let error = model.error {
                Text(error).font(.system(size: 12)).foregroundStyle(.red)
            }
        }
        .padding(16)
        .frame(width: 640)
        .onChange(of: model.focus) { _, wanted in
            if wanted {
                focused = true
                model.focus = false
            }
        }
        .onAppear { focused = true }
    }
}
