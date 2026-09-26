// Local reminders (Section 7: reminders are scheduled locally on each
// device because the server cannot read them). Every store change
// reschedules from the core's reminder field; the Complete and Snooze
// actions go back through the core.

import Foundation
import ListeCore
import ListeKit
import UserNotifications

@MainActor
final class Reminders: NSObject, UNUserNotificationCenterDelegate {
    private let session: Session
    private let center = UNUserNotificationCenter.current()
    private static let category = "liste.reminder"

    init(session: Session) {
        self.session = session
        super.init()
        center.delegate = self
        let complete = UNNotificationAction(identifier: "complete", title: "Complete", options: [])
        let snooze = UNNotificationAction(identifier: "snooze", title: "Snooze 1 hour", options: [])
        center.setNotificationCategories([
            UNNotificationCategory(identifier: Reminders.category, actions: [complete, snooze], intentIdentifiers: [])
        ])
        center.requestAuthorization(options: [.alert, .sound, .badge]) { _, error in
            if let error { log.error("notifications: \(error.localizedDescription)") }
        }
    }

    private var scheduled: [String: Int64] = [:]

    /// Replace every pending reminder with the current set from the core,
    /// when that set changed.
    func reschedule() {
        let tasks: [TaskItem]
        do {
            tasks = try session.query(hasReminder: true)
        } catch {
            return
        }
        var current: [String: Int64] = [:]
        for task in tasks {
            if let at = task.reminderAt { current[task.id] = at }
        }
        guard current != scheduled else { return }
        scheduled = current
        center.removeAllPendingNotificationRequests()
        let now = Date()
        for task in tasks {
            guard let ms = task.reminderAt else { continue }
            let at = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
            guard at > now else { continue }
            let content = UNMutableNotificationContent()
            content.title = task.title.isEmpty ? "Task" : task.title
            if let due = DueDisplay.text(for: task) { content.body = "Due \(due)" }
            content.categoryIdentifier = Reminders.category
            content.userInfo = ["task": task.id]
            content.sound = .default
            let components = Calendar.current.dateComponents([.year, .month, .day, .hour, .minute, .second], from: at)
            let trigger = UNCalendarNotificationTrigger(dateMatching: components, repeats: false)
            center.add(UNNotificationRequest(identifier: "reminder.\(task.id)", content: content, trigger: trigger))
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse
    ) async {
        let id = response.notification.request.content.userInfo["task"] as? String
        let action = response.actionIdentifier
        await MainActor.run {
            guard let id, let task = try? session.task(id: id) else { return }
            switch action {
            case "complete": session.setCompleted(task, true)
            case "snooze": session.setReminder(task, "in 1 hour")
            default: break
            }
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter, willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .sound]
    }
}
