---
title: Review evidence must bind before launch and survive cleanup plus manager adoption
date: 2026-09-28
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/src/services/fleet_review.rs
  - services/hub-rs/src/services/wakes.rs
promoted: false
---

# Review evidence must bind before launch and survive cleanup plus manager adoption

## Observation
The Rust ReviewStore is shared by worktree removal, launch, Wakes and manager transfer. Worktrees capture the base commit before setup hooks; Coordinator registers that exact allocation before engine admission, never a worker-supplied cwd or claimed commit. Cleanup awaits capture under the existing exclusive worktree-maintenance lease. Wakes attaches reviewEvidenceId before TaskStore.validated and rechecks liveness/ownership after Git work. A transferred reader may reuse a retained before-removal capture from the same allocation generation even after the directory and branch are deleted; immutable records keep their original owner. Read and forget accept exact stored selectors only, and invalid file selectors cannot revoke evidence. Windows verbatim prefixes are normalized for identity comparisons without weakening Unix path boundaries.
