---
title: Node 26 Web Storage can mask jsdom localStorage in renderer tests
date: 2026-09-15
confidence: high
related_paths:
  - apps/desktop/src/renderer/vitest.config.ts
  - apps/desktop/src/renderer/tests/setup.ts
  - apps/desktop/src/renderer/tests/components/ClaudePane.test.tsx
promoted: false
---

# Node 26 Web Storage can mask jsdom localStorage in renderer tests

## Observation
On Node v26.2.0, the unmodified renderer test command produced 88 failures across nine existing ClaudePane/useWhatsNew suites: window.localStorage was undefined and Node warned --localstorage-file was not supplied. Re-running ClaudePane.test.tsx and useWhatsNew.test.tsx sequentially with NODE_OPTIONS=--no-experimental-webstorage passed all 53 tests without source changes. This flag lets jsdom provide browser storage in this environment.

## Recommendation
For Node 26 renderer validation, disable experimental Web Storage in the test command; do not interpret storage failures as a product regression before checking the Node environment.
