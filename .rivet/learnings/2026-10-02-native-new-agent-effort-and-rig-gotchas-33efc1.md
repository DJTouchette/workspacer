---
title: Native New Agent effort and rig gotchas
date: 2026-10-02
promoted: false
---

# Native New Agent effort and rig gotchas

## Observation
Effort: Codex rows from providers.listModels carry effortLevels/defaultEffort (claudemon ModelInfo camelCase); Claude has no per-model list, so native uses the --effort launch ladder low..max (desktop CLAUDE_EFFORT_LEVELS). Native reconciles effort on model pick, catalog sync and provider switch (launch.rs reconcile_effort) and sends spawn effort only when chosen. GPUI tests: clicking a gpui-component Select option with simulate_click does not close the popover within the test frame, so later debug_bounds still see the old menu's options; drive Selects by focusing the SelectState and keystrokes (enter / down / enter). Footer text that wraps (long hub errors) under-measures height unless its container is a flex row with a definite width and the panel has a definite height. Visual rig: an agent shell may export WKS_CODEX_BIN/WKS_CLAUDE_BIN pointing at real CLIs; claudemon resolve_provider_bin honours them before PATH, so a private-PATH rig must scrub WKS_* or the hub runs the real codex app-server for model/list. Pre-seeding config.yaml in a fresh --rust-local-dir trips the remote-token STATE LOSS guard; initialize with native-harness rust-probe first, then merge projects.
