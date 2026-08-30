---
title: mobile.html inboxCard assumed every 'stuck' item carries questions[]
date: 2026-08-23
confidence: high
related_paths:
  - services/hub/cmd/hub/mobile.html
promoted: true
promoted_to: remote-mobile
---

# mobile.html inboxCard assumed every 'stuck' item carries questions[]

## Observation
In services/hub/cmd/hub/mobile.html, attentionFor() can emit the 'stuck' kind two ways: (1) a stale pendingQuestions item past STUCK_MS, which carries `it.questions`, and (2) (added for stall detection) a progress-fingerprint stall with no `questions` at all, just title/detail like bigdiff/error. inboxCard()'s render branch used to be `it.kind === 'question' || it.kind === 'stuck'`, which unconditionally rendered the questions-shaped body (`qs2 = it.questions || []`) for BOTH — so a stall card with no questions rendered an empty question block with just a dangling "Decline & stop" button and no visible title/detail text at all. Fixed to `it.kind === 'question' || (it.kind === 'stuck' && it.questions)`, with a new dedicated `it.kind === 'stuck'` (no questions) branch that shows `it.title` bolded above `it.detail` — the generic bigdiff/done branch drops `it.title` in favor of `it.detail` alone, which would have erased the Not-moving/No-signal distinction entirely.

## Impact
Any future attentionFor() item that reuses the 'stuck' kind without a `questions` array will hit this same branch-matching trap in inboxCard() — verify the render branch explicitly, don't assume `kind` alone selects the right body shape.

## Recommendation
When adding a new attention item shape, check inboxCard()'s branch conditions in mobile.html explicitly rather than assuming the `kind` string alone routes it correctly — kinds are shared across differently-shaped payloads (question vs stall-detection both use 'stuck').
