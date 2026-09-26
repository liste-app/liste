// Measurement for the scroll and open checks: frames against the
// display's refresh interval, and main-thread time for a change.

import AppKit
import QuartzCore

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

/// The cost of a change on the main thread: CPU time the thread spends
/// from the call until the run loop next goes idle, which covers the
/// queries, SwiftUI's update, AppKit's layout, and the display pass,
/// but not the time the loop sleeps waiting for the display cycle.
/// Wall time until idle is reported beside it.
@MainActor
enum MainThreadTimer {
    struct Sample {
        var cpuMilliseconds: Double
        var wallMilliseconds: Double
    }

    private static func threadCPU() -> Double {
        Double(clock_gettime_nsec_np(CLOCK_THREAD_CPUTIME_ID)) / 1_000_000
    }

    static func measure(_ change: @MainActor () -> Void) async -> Sample {
        let wallStart = CACurrentMediaTime()
        let cpuStart = threadCPU()
        change()
        return await withCheckedContinuation { (continuation: CheckedContinuation<Sample, Never>) in
            var observer: CFRunLoopObserver?
            observer = CFRunLoopObserverCreateWithHandler(nil, CFRunLoopActivity.beforeWaiting.rawValue, false, CFIndex(Int32.max)) { _, _ in
                let sample = Sample(
                    cpuMilliseconds: threadCPU() - cpuStart,
                    wallMilliseconds: (CACurrentMediaTime() - wallStart) * 1000)
                if let observer { CFRunLoopRemoveObserver(CFRunLoopGetMain(), observer, .commonModes) }
                continuation.resume(returning: sample)
            }
            CFRunLoopAddObserver(CFRunLoopGetMain(), observer, .commonModes)
        }
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
