---
title: cssVarsOf exports no on-accent foreground and no terminal palette as CSS vars
date: 2026-09-02
confidence: high
suggested_doc: theme-system
related_paths:
  - apps/desktop/src/renderer/src/themes.ts
  - apps/desktop/src/renderer/src/App.css
  - apps/desktop/DESIGN_LANGUAGE.md
promoted: false
---

# cssVarsOf exports no on-accent foreground and no terminal palette as CSS vars

## Observation
themes.ts cssVarsOf() emits --wks-* for every flat Theme color plus derived --wks-glass-*, but two things a themed mockup needs are missing: (1) a foreground to paint on a solid accent fill (Dracula lilac #bd93f9 needs dark text, Light blue #2563eb needs white; DESIGN_LANGUAGE.md allows raw #000/#fff, which can only be right for one of them), and (2) the Theme.terminal ANSI palette (cyan/magenta/yellow) is never exported as CSS vars, so inline code in prose cannot match the terminal without a literal. The 2026-09-02 direction C mockup (.workspacer/design/2026-09-02/c-editor-native, gitignored) added --wks-accent-fg per theme and --wks-term-cyan/-magenta/-yellow, plus derived --wks-bg-statusbar / --wks-bg-activitybar / --wks-keycap-* / --wks-prefix-armed.

## Impact
Any redesign that puts text on accent fills or wants inline code to match the terminal palette will hard-code a color unless these tokens exist.

## Recommendation
If the editor-native direction (or any of the three) is adopted, add accentFg (optional, defaulting via isLightTheme) to Theme and export terminal.cyan/magenta/yellow from cssVarsOf; keep the statusbar/activitybar/keycap tokens as color-mix derivations in App.css :root so all 18 themes work unchanged.
