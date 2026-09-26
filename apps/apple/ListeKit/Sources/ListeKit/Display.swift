// Display formatting only: how a due date reads, whether it is overdue.
// The instant comes from the core; this never decides what a date means.

import Foundation
import ListeCore

public enum DueDisplay {
    /// "Today", "Tomorrow", "Yesterday", a weekday within the week, else the
    /// absolute date; with the time when the task is not all-day.
    public static func text(for task: TaskItem, now: Date = Date()) -> String? {
        guard let ms = task.dueAt else { return nil }
        let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        let calendar = Calendar.current
        let day: String
        if calendar.isDateInToday(date) {
            day = "Today"
        } else if calendar.isDateInTomorrow(date) {
            day = "Tomorrow"
        } else if calendar.isDateInYesterday(date) {
            day = "Yesterday"
        } else if let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: now), to: calendar.startOfDay(for: date)).day,
            days > 1, days < 7
        {
            day = date.formatted(.dateTime.weekday(.wide))
        } else if calendar.component(.year, from: date) == calendar.component(.year, from: now) {
            day = date.formatted(.dateTime.month(.abbreviated).day())
        } else {
            day = date.formatted(.dateTime.year().month(.abbreviated).day())
        }
        if task.dueAllDay {
            return day
        }
        return "\(day) \(date.formatted(.dateTime.hour().minute()))"
    }

    /// Overdue: due before now (before today for all-day) and not completed.
    public static func isOverdue(_ task: TaskItem, now: Date = Date()) -> Bool {
        guard task.completedAt == nil, let ms = task.dueAt else { return false }
        let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        if task.dueAllDay {
            return date < Calendar.current.startOfDay(for: now)
        }
        return date < now
    }

    /// The section heading for a due day in Upcoming.
    public static func dayHeading(for ms: Int64?, now: Date = Date()) -> String {
        guard let ms else { return "No date" }
        let date = Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return "Today" }
        if calendar.isDateInTomorrow(date) { return "Tomorrow" }
        return date.formatted(.dateTime.weekday(.wide).month(.abbreviated).day())
    }

    /// Tasks grouped by due day, in order, for Upcoming.
    public static func byDay(_ tasks: [TaskItem]) -> [(heading: String, tasks: [TaskItem])] {
        let calendar = Calendar.current
        var groups: [(Date?, [TaskItem])] = []
        for task in tasks {
            let day = task.dueAt.map { calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval($0) / 1000)) }
            if let last = groups.indices.last, groups[last].0 == day {
                groups[last].1.append(task)
            } else {
                groups.append((day, [task]))
            }
        }
        return groups.map { (dayHeading(for: $0.0.map { Int64($0.timeIntervalSince1970 * 1000) }), $0.1) }
    }

    public static func absolute(_ ms: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(ms) / 1000).formatted(date: .abbreviated, time: .shortened)
    }
}
