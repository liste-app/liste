# design

Shared design tokens: colors, spacing, radii, and the type scale. Tokens are the source of truth here and are exported to each platform's format (Tailwind theme variables on web; Swift, Kotlin, XAML, and GTK CSS resources on native).

Liste has its own design language that adapts to each platform's conventions. Share tokens, not component implementations; what must not drift is behavior, not pixels (Section 17 of `docs/ARCHITECTURE.md`).

`tokens.json` is the source. `generate.py` (or `just tokens`) emits `apps/apple/ListeKit/Sources/ListeKit/Tokens.swift` and `apps/web/src/lib/tokens.css`; both generated files are committed so a change to a token is visible in review on every platform at once.
