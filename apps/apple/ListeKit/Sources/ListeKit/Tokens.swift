// Generated from design/tokens.json by design/generate.py. Do not edit; change the tokens and regenerate.

import SwiftUI

#if canImport(AppKit)
    import AppKit
#endif

/// Liste's design tokens (Section 17): the only colors, sizes, and
/// durations the app uses, plus system fonts.
public enum Tokens {
    public enum Colors {
        /// The system accent color; the brand accent is a placeholder until the logo work lands.
        public static let accent = Color.accentColor
        public static let background = dynamic(light: (1.0000, 1.0000, 1.0000), dark: (0.1176, 0.1176, 0.1255))
        public static let backgroundSecondary = dynamic(light: (0.9608, 0.9608, 0.9686), dark: (0.1647, 0.1647, 0.1765))
        public static let backgroundTertiary = dynamic(light: (0.9216, 0.9216, 0.9373), dark: (0.2078, 0.2078, 0.2196))
        public static let text = dynamic(light: (0.1137, 0.1137, 0.1216), dark: (0.9490, 0.9490, 0.9569))
        public static let textSecondary = dynamic(light: (0.4314, 0.4314, 0.4510), dark: (0.6314, 0.6314, 0.6510))
        public static let textTertiary = dynamic(light: (0.6824, 0.6824, 0.6980), dark: (0.4314, 0.4314, 0.4510))
        public static let separator = dynamic(light: (0.8863, 0.8863, 0.9020), dark: (0.2353, 0.2353, 0.2510))
        public static let selection = dynamic(light: (0.8667, 0.9059, 0.9843), dark: (0.1843, 0.2471, 0.3725))
        public static let overdue = dynamic(light: (0.7843, 0.2118, 0.1843), dark: (0.9412, 0.3961, 0.3569))
        public static let success = dynamic(light: (0.1804, 0.5451, 0.3412), dark: (0.2980, 0.7333, 0.4784))
        public static let priorityHigh = dynamic(light: (0.7843, 0.2118, 0.1843), dark: (0.9412, 0.3961, 0.3569))
        public static let priorityMedium = dynamic(light: (0.8510, 0.5098, 0.1686), dark: (0.9490, 0.6353, 0.3059))
        public static let priorityLow = dynamic(light: (0.2314, 0.4353, 0.8784), dark: (0.3569, 0.5529, 0.9373))
        public static let spanDate = dynamic(light: (0.2314, 0.4353, 0.8784), dark: (0.3569, 0.5529, 0.9373))
        public static let spanTime = dynamic(light: (0.0549, 0.5490, 0.6039), dark: (0.2471, 0.7216, 0.7765))
        public static let spanList = dynamic(light: (0.4784, 0.3098, 0.8196), dark: (0.6392, 0.5098, 0.9333))
        public static let spanTag = dynamic(light: (0.1804, 0.5451, 0.3412), dark: (0.2980, 0.7333, 0.4784))
        public static let spanPriority = dynamic(light: (0.8510, 0.5098, 0.1686), dark: (0.9490, 0.6353, 0.3059))
        public static let spanRecurrence = dynamic(light: (0.7608, 0.2314, 0.5412), dark: (0.8980, 0.4235, 0.7020))

        /// A color that follows the system appearance.
        static func dynamic(light: (Double, Double, Double), dark: (Double, Double, Double)) -> Color {
            #if canImport(AppKit)
                return Color(nsColor: NSColor(name: nil) { appearance in
                    let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
                    let (r, g, b) = isDark ? dark : light
                    return NSColor(srgbRed: r, green: g, blue: b, alpha: 1)
                })
            #else
                return Color(uiColor: UIColor { traits in
                    let (r, g, b) = traits.userInterfaceStyle == .dark ? dark : light
                    return UIColor(red: r, green: g, blue: b, alpha: 1)
                })
            #endif
        }

        /// The highlight color for a capture span kind.
        public static func span(_ kind: String) -> Color {
            switch kind {
            case "date": spanDate
            case "time": spanTime
            case "list": spanList
            case "tag": spanTag
            case "priority": spanPriority
            case "recurrence": spanRecurrence
            default: textSecondary
            }
        }

        /// The color for a priority name.
        public static func priority(_ name: String) -> Color {
            switch name {
            case "high": priorityHigh
            case "medium": priorityMedium
            case "low": priorityLow
            default: textTertiary
            }
        }
    }

    public enum Space {
        public static let xxs: CGFloat = 2
        public static let xs: CGFloat = 4
        public static let sm: CGFloat = 8
        public static let md: CGFloat = 12
        public static let lg: CGFloat = 16
        public static let xl: CGFloat = 24
        public static let xxl: CGFloat = 32
    }

    public enum Radius {
        public static let sm: CGFloat = 4
        public static let md: CGFloat = 6
        public static let lg: CGFloat = 10
        public static let xl: CGFloat = 14
    }

    /// System fonts at the token text styles, so dynamic type applies.
    public enum Typography {
        public static let caption = Font.system(.caption, weight: .regular)
        public static let footnote = Font.system(.footnote, weight: .regular)
        public static let body = Font.system(.body, weight: .regular)
        public static let bodyStrong = Font.system(.body, weight: .semibold)
        public static let callout = Font.system(.callout, weight: .regular)
        public static let title = Font.system(.title3, weight: .semibold)
        public static let largeTitle = Font.system(.title, weight: .bold)
        public static let capture = Font.system(.title2, weight: .regular)
    }

    /// Durations in seconds. `animation` returns nil under reduced motion.
    public enum Motion {
        public static let fast: Double = 0.120
        public static let normal: Double = 0.200
        public static let slow: Double = 0.320

        public static var reduced: Bool {
            #if canImport(AppKit)
                NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            #else
                UIAccessibility.isReduceMotionEnabled
            #endif
        }

        public static func animation(_ duration: Double) -> Animation? {
            reduced ? nil : .easeOut(duration: duration)
        }
    }
}
