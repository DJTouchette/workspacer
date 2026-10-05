---
title: Native finished child rows flickered from a busy-parent filter and a fleet overlay gap
date: 2026-10-05
confidence: high
suggested_doc: session-lifecycle
related_paths:
  - apps/native/src/ui/sidebar.rs
  - apps/native/src/controller.rs
  - apps/native/src/model.rs
promoted: false
---

# Native finished child rows flickered from a busy-parent filter and a fleet overlay gap

## Observation
Issue #29 had two causes. (1) sidebar_rows showed a settled provider-native child only while the parent was working/approval/question or while that child was the open ChildTarget, so a finished child vanished at turn end and reappeared whenever it was opened from the chat. (2) Controller fleet_overlay (agent.snapshot events seen while a sessions.snapshots read is pending) copied label/mode/pending but not subagents/workflows, and Completion::Fleet does sessions.clear() then upserts rows then the overlay; an older fleet row therefore rolled a finished child back to running, and dropped a newly started one, until the next event (every 30s refresh). Daemon/hub keep every child (store appends, max 128; hub rows are full GET /sessions/:id), so no server change was needed. Native Session.merge also kept the FIRST 32 children, hiding the newest/running ones past 32; it now keeps the newest 32.

## Impact
Removing the busy-parent filter alone leaves the 30s fleet-refresh flash; the overlay fix alone keeps the documented disappearance. Clear is device-local view state in native-settings cleared_children[hub scope][agent:<parent>/<id> | session:<id>] -> ClearMark{clearedAtMs,startedAtMs,completedAtMs,toolCalls}; it is NOT sessionArchive and sends no command.

## Recommendation
Hide a cleared child only while it is settled and ClearMark::covers(current evidence) holds (unknown values are not new work, so replays/backfilled telemetry never resurrect it); lift the mark when the child is observed running/working. Keep debug_bounds absence assertions on a window's first frame: GPUI never clears rendered_frame.debug_bounds, so stale selectors survive re-renders.
