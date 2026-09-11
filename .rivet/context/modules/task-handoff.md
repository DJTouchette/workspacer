---
title: Exact workspace task handoff and output custody
tags: [handoff, git, artifacts, paired, dispatch, security]
related_paths:
  - apps/desktop/src/main/services/taskHandoff.ts
  - apps/desktop/src/main/services/pairedDispatch.ts
  - services/hub/cmd/brain/taskhandoff.go
  - services/hub/cmd/brain/dispatchlease.go
  - services/hub/internal/taskartifacts
---

The user-selected code transport is an explicitly approved Git remote, with
selected artifact bytes on the existing paired bus. See
`docs/workspace-handoff.md` for configuration, supported bounds and entry points.

The brain is the single file owner on each host, including desktop catalog
mode. `agents.taskHandoff` therefore belongs in BOTH `methods()` and
`catalogMethods()`. The real MCP fixture registers the production catalog
handler on the desktop's bus; testing only a helper or the execution host misses
the default route.

Custody must be validated independently of a live manager and message delivery.
`acceptHandoffUpdate` validates the peer/task/session without granting a wake;
`accept` retains the existing live-manager and unknown-delivery gate. A text ACK
cannot evict transfer custody or trigger cleanup. Dirty output is a retained
checkpoint requirement, not a successful import.

A local predecessor may have committed in its own worktree while the configured
project checkout remains elsewhere. Resolve the owned predecessor and compare
Git common-directory identity, then pin its exact commit. For an imported
predecessor, use the binding-scoped custody receipt instead of a producer path.
Generated source pins preserve objects if the original workspace is removed.

Git quarantine and review are separate from the user's active tree. Object
promotion uses an internal verified repository, disables FETCH_HEAD writes, and
uses compare-and-swap for the generated origin review ref. Never replace it with
pull, active-branch checkout, automatic staging, or inherited peer hooks/config.

The existing parity test parses the desktop spawn type with a regex: inline
nested object types at its start terminate that parser. Use the named
`HandoffReceiptSelector` type. The capspec witness checker requires short
parameter names such as `cwd` to be quoted with backticks in its rationale.

Hosted macOS temp paths can use `/var` aliases for `/private/var`; tests that
compare raw temporary paths need a canonical test temp root. Windows requires
real ACL checks and Git long-path handling, not assertions about synthetic POSIX
mode bits. No personal target is needed for these fixtures.
