---
title: Second-pass source audit invalidates historical runtime guarantees
date: 2026-09-26
confidence: high
suggested_doc: renderer-backend-seam
promoted: false
---

# Second-pass source audit invalidates historical runtime guarantees

## Observation
Current backend now forwards demanded conversation deltas and maps statusLine/totalToolCalls; workflow run reads use desktop services and rich handoff also exists in Go. Nightly publish deletes the old release before its final publish and is not atomic. Embedded watch refresh does not clear its gate mirror and its one-second timer is recreated per select loop. Claude root caps do not bound preceding directory scans, and description parsing reads the whole file before slicing.

## Impact
Append-only historical fixes can contradict executable behavior even when all referenced paths and lint pass.

## Recommendation
Keep current contracts consolidated; verify function bodies rather than source comments. Preserve explicit controlled-test and platform limits.
