---
title: GPUI debug bounds retain selectors after elements disappear
date: 2026-10-01
promoted: false
---

# GPUI debug bounds retain selectors after elements disappear

## Observation
GPUI 0.2.2 Frame::clear clears scene and element state but does not clear debug_bounds. VisualTestContext::debug_bounds can therefore retain a selector after its element was removed. Test transitions with a newly painted state marker and actual controller ownership/requests rather than expecting a previously painted selector to disappear.
