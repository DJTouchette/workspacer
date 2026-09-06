---
title: Codex app-server direct-child teardown leaves owned descendants
date: 2026-09-06
confidence: high
suggested_doc: claudemon-providers
related_paths:
  - services/claudemon/src/providers/codex.rs
  - services/claudemon/src/session/store.rs
promoted: false
---

# Codex app-server direct-child teardown leaves owned descendants

## Observation
The isolated Linux fixture direct_child_cleanup_gap_fixture kills and reaps only its direct Python child and observes the forked descendant surviving. OwnedAppServer now establishes a Unix process group and observes direct exit with waitid WNOWAIT before signaling the group, so reaping cannot release the PID anchor before killpg. Separate fixture subprocesses adopt and reap descendants; GroupStillPresent is a cleanup failure when a descendant has not yet been reaped. This proves the local gap, not the unknown historic orphan cause.

## Impact
Direct Child kill_on_drop or start_kill does not imply descendant termination. A stale generation must clean its own handle but must not deregister or replace a successor.

## Recommendation
Review docs/reviews/codex-driver-cleanup.md and run the isolated fixtures. GroupGone covers members remaining in the created group; it does not prove containment of descendants that call setsid or change groups. Windows per-driver jobs remain pending.
