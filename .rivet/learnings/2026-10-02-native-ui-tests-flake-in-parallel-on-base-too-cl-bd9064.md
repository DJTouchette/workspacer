---
title: Native ui-tests flake in parallel on base too (click/geometry, not this branch)
date: 2026-10-02
confidence: medium
related_paths:
  - apps/native/src/ui.rs
promoted: false
---

# Native ui-tests flake in parallel on base too (click/geometry, not this branch)

## Observation
cargo test --features ui-tests --bin wks-native in parallel fails ~4/6 runs: guided_launch_keeps_options_and_start_action_accessible (clicks on launch-customize not registering, ui.rs:3941/3963), keyboard_controls_skip_disabled_actions_and_keep_their_geometry (sidebar-toggle bounds move 27px between frames, ui.rs:4250), occasionally markdown_file_link_requests_preview_and_shows_the_result. Reproduced on 9d4411ef in a clean worktree, so pre-existing; --test-threads=1 passes 101/101.

## Impact
A parallel red run is not evidence against a change; geometry assertions and simulated clicks there depend on something shared or real-time across concurrent gpui tests (root cause not found).

## Recommendation
Gate on the serialized UI run until the shared state is found; suspect real-time animation or process-shared text/platform state.
