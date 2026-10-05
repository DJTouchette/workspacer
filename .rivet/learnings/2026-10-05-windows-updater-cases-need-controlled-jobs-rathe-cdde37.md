---
title: Windows updater cases need controlled jobs rather than inherited runner policy
date: 2026-10-05
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/native/tests/windows_update_handoff.rs
promoted: false
---

# Windows updater cases need controlled jobs rather than inherited runner policy

## Observation
On e23a2375 Windows CI, the explicit BREAKAWAY_OK success case passed but default-child and direct-driver cases failed immediately with access denied before testing installer behavior. Those cases inherited the runner job. Each case now runs in its own gated subprocess assigned a permissive kill-on-close job before case code executes; the denied-breakaway scenario adds its deliberately restrictive inner job. All six behavioral assertions and the exact CI six-pass gate remain. Failed cases retain and print handoff error, state and helper logs.
