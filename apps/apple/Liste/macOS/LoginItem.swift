// Launch at login (Section 3): on by default, off by a toggle.

import Foundation
import ServiceManagement

@MainActor
enum LoginItem {
    private static let key = "launchAtLogin"

    static var isEnabled: Bool {
        SMAppService.mainApp.status == .enabled
    }

    /// The first run registers the login item; later runs respect the
    /// person's choice.
    static func registerOnFirstRun() {
        let defaults = UserDefaults.standard
        guard defaults.object(forKey: key) == nil else { return }
        setEnabled(true)
    }

    static func setEnabled(_ enabled: Bool) {
        UserDefaults.standard.set(enabled, forKey: key)
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            log.error("launch at login: \(error.localizedDescription)")
        }
    }
}
