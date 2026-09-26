// Frame-rate measurement for the scroll check (debug builds). Counts
// display-link callbacks against the display's refresh interval.

import AppKit
import QuartzCore

#if DEBUG
    @MainActor
    final class FrameMeter {
        private var link: CADisplayLink?
        private var frames = 0
        private var dropped = 0
        private var last: CFTimeInterval = 0
        private var started: CFTimeInterval = 0

        func start() {
            frames = 0
            dropped = 0
            last = 0
            started = CACurrentMediaTime()
            let link = NSScreen.main?.displayLink(target: self, selector: #selector(tick(_:)))
            link?.add(to: .main, forMode: .common)
            self.link = link
        }

        @objc private func tick(_ link: CADisplayLink) {
            frames += 1
            if last > 0 {
                let interval = link.targetTimestamp - link.timestamp
                let elapsed = link.timestamp - last
                if elapsed > interval * 1.5 {
                    dropped += Int((elapsed / interval).rounded()) - 1
                }
            }
            last = link.timestamp
        }

        func stop() -> String {
            link?.invalidate()
            link = nil
            let seconds = CACurrentMediaTime() - started
            let fps = Double(frames) / max(seconds, 0.001)
            return String(format: "%.1f fps over %.1f s, %d frames, %d dropped", fps, seconds, frames, dropped)
        }
    }

    extension NSView {
        /// The first scroll view in the hierarchy, depth first.
        func firstScrollView() -> NSScrollView? {
            if let s = self as? NSScrollView { return s }
            for sub in subviews {
                if let s = sub.firstScrollView() { return s }
            }
            return nil
        }
    }
#endif
