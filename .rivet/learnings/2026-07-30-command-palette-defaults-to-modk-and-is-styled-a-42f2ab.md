---
title: Command palette defaults to mod+K and is styled as the app's front door
date: 2026-07-30
promoted: true
---

# Command palette defaults to mod+K and is styled as the app's front door

## Observation
Default binding moved from mod+shift+p to mod+k in TWO places that must agree — services/hub/cmd/brain/config_defaults.json (the shipped keybindings, regenerated into both configDefaults.generated.ts) and the VSCODE preset in lib/keybindingPresets.ts (re-picking a preset overwrites the config, so a mismatch shows up only after a preset switch); a test now pins DEFAULT_CONFIG and KEYBINDING_PRESETS[DEFAULT_PRESET_ID] to the same value. Vim ('prefix p') and JetBrains ('mod+shift+a') presets keep their native bindings — an action carries exactly ONE combo (eventMatchesCombo/buildDirectMatchers parse a single string), so alternates would be a real feature, not a config tweak. Restyle: 620px wide, 52px borderless search row led by the BrandMark, esc chip, accent-tinted selection with an inset left rule, unified footer (nav hints, or library verbs when library rows are showing, plus a result count). GOTCHA: commandPaletteNav.test.tsx found rows by exact CSS (style.gap === '10px', backgroundColor === var(--wks-bg-selected)), so the restyle broke a test whose behaviour was intact — rows now carry data-palette-row/data-selected and the test reads those.

## Disposition
Folded into .rivet/context/domains/config.md (defaults-twins note: config_defaults.json + keybindingPresets VSCODE preset). The palette-restyle/test-selector detail was left as historical.
