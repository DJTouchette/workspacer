---
title: Node25 Web Storage can break baseline jsdom renderer tests
date: 2026-09-13
confidence: high
related_paths:
  - apps/desktop/src/renderer/tests/setup.ts
  - apps/desktop/src/renderer/vitest.config.ts
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
promoted: false
---

# Node25 Web Storage can break baseline jsdom renderer tests

## Observation
On local Node v25.1.0, full desktop main tests pass but 88 renderer tests in nine baseline suites fail because global localStorage.getItem/clear are not functions (Node warns --localstorage-file lacks a valid path). The affected production ClaudePane and renderer setup were untouched. Running representative ClaudePane + useWhatsNew suites with command-only NODE_OPTIONS=--no-webstorage passes all53 tests. Use the project's Node22 CI runtime for release parity, or disable Node25's native Web Storage for local jsdom test execution rather than modifying product code to mask it.
