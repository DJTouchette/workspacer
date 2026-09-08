---
title: An adjacent pilot anchor produces no visible navigation on a tall desktop
date: 2026-09-07
confidence: high
related_paths:
  - landing/enterprise.html
promoted: false
---

# An adjacent pilot anchor produces no visible navigation on a tall desktop

## Observation
At 1440x1200 the original enterprise page fit the viewport and #pilot sat beside the hero. Clicking its hero link changed the hash but scrollY stayed zero. A lower standalone pilot section makes the destination visibly distinct; test actual scroll displacement on tall desktop as well as mobile.

## Recommendation
Keep the pilot destination after the screenshot tour and verify a visible destination change, rather than checking only fragment resolution.
