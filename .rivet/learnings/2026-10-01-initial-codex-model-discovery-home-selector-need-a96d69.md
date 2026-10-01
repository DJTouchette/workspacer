---
title: Initial Codex model discovery home selector needs source policy review
date: 2026-10-01
promoted: false
---

# Initial Codex model discovery home selector needs source policy review

## Observation
The b06a333a native Codex initial model discovery change added providers.listModels.useHomeDirectory, a codex-only literal boolean used with an empty cwd to select and canonicalize the hub-owned home. The capability source guard correctly reported an unreviewed dangerous spelling; add a per-method sourceParameterDecisions path review, not a global vocabulary admission. This selector never supplies a caller home path, overrides no explicit project cwd, and does not participate in agents.spawn. Scanner passes with 122 methods, 123 dangerous bindings and 23 opaque methods after this narrow contract review.
