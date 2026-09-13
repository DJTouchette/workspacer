---
title: Intent shell layout validation covers compact list and keyboard tab scrolling
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/tests/e2e/intentCompletion.test.ts
promoted: false
---

# Intent shell layout validation covers compact list and keyboard tab scrolling

## Observation
Production App with an isolated real headless intent store now has a browser layout matrix at 320/768/1280 pixels under both dark/light themes. It verifies mobile Work list expansion and closing on correct work selection; all nine views stay in a single horizontal scrolling tab row; Home/End/Arrow wrap updates focus, selected state, roving tabstop, and associated tabpanel; the selected tab scrolls fully into view; no detail/viewport horizontal overflow. Full record/artifact/evidence/knowledge/control workflow remains a separate real-host test with fake session transport only.
