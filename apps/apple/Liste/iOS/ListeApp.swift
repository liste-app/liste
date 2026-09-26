// Liste for iOS (Phase 1). The iOS target shares ListeKit with the macOS
// app; it is not built yet. On iOS there is no host process: the app links
// the core in-process and is the only thing on the device that touches the
// store (Section 3).

import SwiftUI

@main
struct ListeApp: App {
    var body: some Scene {
        WindowGroup {
            Text("Liste")
        }
    }
}
