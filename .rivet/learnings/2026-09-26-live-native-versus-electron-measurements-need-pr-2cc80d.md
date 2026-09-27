---
title: Live native versus Electron measurements need process and workload boundaries
date: 2026-09-26
confidence: high
related_paths:
  - apps/native/scripts/compare-clients.py
  - apps/native/README.md
promoted: false
---

# Live native versus Electron measurements need process and workload boundaries

## Observation
A 20-second Linux sample of the simultaneously running native release client and Electron dev host measured mean PSS 241.8 MiB vs 1318.2 MiB and CPU 1.89% vs 140.8% of one core. Electron included its 8 Chromium processes, open DevTools, and in-process host services; native was the smaller client. Separate hub/claudemon/build tools were excluded. This is a current-workload footprint comparison, not a controlled UI speedup or projected whole-stack saving. apps/native/scripts/compare-clients.py now repeats the measurement and reports scope; use PSS to avoid shared-page double counting.

## Recommendation
For a defensible architecture comparison, run matching release builds against the same backend, conversation, visibility, and workload; measure GPU memory and responsiveness separately.
