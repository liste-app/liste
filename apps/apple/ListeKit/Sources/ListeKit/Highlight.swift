// Turns the parser's byte spans into highlighted text for the capture
// panel. Offsets from the core are UTF-8 byte offsets into the exact
// string that was parsed; they are mapped back onto the Swift string here
// and nowhere else.

import Foundation
import ListeCore
import SwiftUI

/// The highlight color for one kind of interpreted span.
public func highlightColor(for kind: String) -> Color {
    switch kind {
    case "date": .blue
    case "time": .teal
    case "list": .purple
    case "tag": .green
    case "priority": .orange
    case "recurrence": .pink
    default: .gray
    }
}

/// `text` with each span's background tinted by its kind.
public func highlighted(_ text: String, spans: [Span]) -> AttributedString {
    var attributed = AttributedString(text)
    let utf8 = text.utf8
    for span in spans {
        guard span.start < span.end, Int(span.end) <= utf8.count else { continue }
        let lower = utf8.index(utf8.startIndex, offsetBy: Int(span.start))
        let upper = utf8.index(utf8.startIndex, offsetBy: Int(span.end))
        guard let start = AttributedString.Index(lower, within: attributed),
            let end = AttributedString.Index(upper, within: attributed)
        else { continue }
        attributed[start..<end].backgroundColor = highlightColor(for: span.kind).opacity(0.25)
        attributed[start..<end].foregroundColor = highlightColor(for: span.kind)
    }
    return attributed
}

/// A one-line summary of what a preview would create, for the panel.
public func summary(of preview: CapturePreview) -> String {
    var parts: [String] = []
    if let due = preview.due {
        parts.append(preview.dueAllDay ? "due \(due)" : "due \(due)")
    }
    if let list = preview.list {
        parts.append("in \(list)")
    }
    if !preview.tags.isEmpty {
        parts.append(preview.tags.map { "#\($0)" }.joined(separator: " "))
    }
    if preview.priority != "none" {
        parts.append("!\(preview.priority)")
    }
    if let rule = preview.recurrence {
        parts.append("repeats \(rule)")
    }
    return parts.joined(separator: " · ")
}
