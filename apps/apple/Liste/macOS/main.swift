// Liste for macOS: AppKit lifecycle so the app controls its own windows.
// SwiftUI draws every view; AppKit decides when a window exists, which is
// what background operation (Section 3) needs.

import AppKit

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
