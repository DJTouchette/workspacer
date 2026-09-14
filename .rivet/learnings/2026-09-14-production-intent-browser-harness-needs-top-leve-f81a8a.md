---
title: Production Intent browser harness needs top-level-await target and isolated provider fetch
date: 2026-09-14
confidence: high
related_paths:
  - apps/desktop/tests/e2e/intentCompletion.test.ts
promoted: false
---

# Production Intent browser harness needs top-level-await target and isolated provider fetch

## Observation
The first-use App harness uses top-level await for dynamic App/config imports. Building that harness with Vite's default production target fails before browser startup. The Intent completion Playwright suite now builds only its test harness with target esnext and serves the optimized output, while production desktop/web builds keep their normal target. The owner process receives a synthetic fetch fixture and isolated environment, so tracker attach runs through real headless services/SQLite without accessing providers or inherited credentials.
