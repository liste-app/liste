#!/usr/bin/env python3
"""Emit platform token files from design/tokens.json.

Apple: apps/apple/ListeKit/Sources/ListeKit/Tokens.swift
Web:   apps/web/src/lib/tokens.css

Run from the repository root: `python3 design/generate.py` or `just tokens`.
"""
import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOKENS = ROOT / "design" / "tokens.json"
SWIFT_OUT = ROOT / "apps" / "apple" / "ListeKit" / "Sources" / "ListeKit" / "Tokens.swift"
CSS_OUT = ROOT / "apps" / "web" / "src" / "lib" / "tokens.css"

HEADER = "Generated from design/tokens.json by design/generate.py. Do not edit; change the tokens and regenerate."


def hex_rgb(value: str) -> tuple[float, float, float]:
    v = value.lstrip("#")
    return tuple(int(v[i : i + 2], 16) / 255 for i in (0, 2, 4))


def kebab(name: str) -> str:
    return re.sub(r"([a-z0-9])([A-Z])", r"\1-\2", name).lower()


def swift(tokens: dict) -> str:
    out = [f"// {HEADER}", "", "import SwiftUI", "", "#if canImport(AppKit)", "    import AppKit", "#endif", "",
           "/// Liste's design tokens (Section 17): the only colors, sizes, and", "/// durations the app uses, plus system fonts.",
           "public enum Tokens {"]
    out.append("    public enum Colors {")
    for name, c in tokens["color"].items():
        if c.get("system"):
            out.append(f"        /// The system accent color; the brand accent is a placeholder until the logo work lands.")
            out.append(f"        public static let {name} = Color.accentColor")
            continue
        lr, lg, lb = hex_rgb(c["light"])
        dr, dg, db = hex_rgb(c["dark"])
        out.append(f"        public static let {name} = dynamic(light: ({lr:.4f}, {lg:.4f}, {lb:.4f}), dark: ({dr:.4f}, {dg:.4f}, {db:.4f}))")
    out += ["",
            "        /// A color that follows the system appearance.",
            "        static func dynamic(light: (Double, Double, Double), dark: (Double, Double, Double)) -> Color {",
            "            #if canImport(AppKit)",
            "                return Color(nsColor: NSColor(name: nil) { appearance in",
            "                    let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua",
            "                    let (r, g, b) = isDark ? dark : light",
            "                    return NSColor(srgbRed: r, green: g, blue: b, alpha: 1)",
            "                })",
            "            #else",
            "                return Color(uiColor: UIColor { traits in",
            "                    let (r, g, b) = traits.userInterfaceStyle == .dark ? dark : light",
            "                    return UIColor(red: r, green: g, blue: b, alpha: 1)",
            "                })",
            "            #endif",
            "        }",
            "",
            "        /// The highlight color for a capture span kind.",
            "        public static func span(_ kind: String) -> Color {",
            "            switch kind {",
            "            case \"date\": spanDate",
            "            case \"time\": spanTime",
            "            case \"list\": spanList",
            "            case \"tag\": spanTag",
            "            case \"priority\": spanPriority",
            "            case \"recurrence\": spanRecurrence",
            "            default: textSecondary",
            "            }",
            "        }",
            "",
            "        /// The color for a priority name.",
            "        public static func priority(_ name: String) -> Color {",
            "            switch name {",
            "            case \"high\": priorityHigh",
            "            case \"medium\": priorityMedium",
            "            case \"low\": priorityLow",
            "            default: textTertiary",
            "            }",
            "        }",
            "    }", ""]
    out.append("    public enum Space {")
    for name, v in tokens["space"].items():
        out.append(f"        public static let {name}: CGFloat = {v}")
    out += ["    }", "", "    public enum Radius {"]
    for name, v in tokens["radius"].items():
        out.append(f"        public static let {name}: CGFloat = {v}")
    out += ["    }", "", "    /// System fonts at the token text styles, so dynamic type applies.", "    public enum Typography {"]
    weights = {"regular": ".regular", "medium": ".medium", "semibold": ".semibold", "bold": ".bold"}
    for name, t in tokens["type"].items():
        if name.startswith("$"):
            continue
        out.append(f"        public static let {name} = Font.system(.{t['style']}, weight: {weights[t['weight']]})")
    out += ["    }", "", "    /// Durations in seconds. `animation` returns nil under reduced motion.", "    public enum Motion {"]
    for name, v in tokens["motion"].items():
        if name.startswith("$"):
            continue
        out.append(f"        public static let {name}: Double = {v / 1000:.3f}")
    out += ["",
            "        public static var reduced: Bool {",
            "            #if canImport(AppKit)",
            "                NSWorkspace.shared.accessibilityDisplayShouldReduceMotion",
            "            #else",
            "                UIAccessibility.isReduceMotionEnabled",
            "            #endif",
            "        }",
            "",
            "        public static func animation(_ duration: Double) -> Animation? {",
            "            reduced ? nil : .easeOut(duration: duration)",
            "        }",
            "    }",
            "}", ""]
    return "\n".join(out)


def css(tokens: dict) -> str:
    out = [f"/* {HEADER} */", ":root {"]
    for name, c in tokens["color"].items():
        out.append(f"  --color-{kebab(name)}: {c['light']};")
    for name, v in tokens["space"].items():
        out.append(f"  --space-{name}: {v}px;")
    for name, v in tokens["radius"].items():
        out.append(f"  --radius-{name}: {v}px;")
    for name, t in tokens["type"].items():
        if name.startswith("$"):
            continue
        out.append(f"  --type-{kebab(name)}-size: {t['size']}px;")
        out.append(f"  --type-{kebab(name)}-line-height: {t['lineHeight']}px;")
        weight = {"regular": 400, "medium": 500, "semibold": 600, "bold": 700}[t["weight"]]
        out.append(f"  --type-{kebab(name)}-weight: {weight};")
    for name, v in tokens["motion"].items():
        if name.startswith("$"):
            continue
        out.append(f"  --motion-{name}: {v}ms;")
    out += ["}", "", "@media (prefers-color-scheme: dark) {", "  :root {"]
    for name, c in tokens["color"].items():
        out.append(f"    --color-{kebab(name)}: {c['dark']};")
    out += ["  }", "}", "", "@media (prefers-reduced-motion: reduce) {", "  :root {"]
    for name, v in tokens["motion"].items():
        if name.startswith("$"):
            continue
        out.append(f"    --motion-{name}: 0ms;")
    out += ["  }", "}", ""]
    return "\n".join(out)


def main() -> None:
    tokens = json.loads(TOKENS.read_text())
    SWIFT_OUT.parent.mkdir(parents=True, exist_ok=True)
    CSS_OUT.parent.mkdir(parents=True, exist_ok=True)
    SWIFT_OUT.write_text(swift(tokens))
    CSS_OUT.write_text(css(tokens))
    print(f"wrote {SWIFT_OUT.relative_to(ROOT)} and {CSS_OUT.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
