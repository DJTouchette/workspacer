---
title: Reconcile native floating chat through main rich transcript renderer
date: 2026-09-29
confidence: high
suggested_doc: chat-tool-rendering
related_paths:
  - apps/native/src/ui.rs
  - apps/native/src/ui/transcript.rs
  - apps/native/src/model.rs
  - apps/native/Cargo.lock
promoted: false
---

# Reconcile native floating chat through main rich transcript renderer

## Observation
The 0c89afff master polish and main's rich transcript implementation both replace native tool rows and scrolling. Integration must retain main's transcript::Tool representation, history bounds, structured response actions, attachment previews and turn_changes summary-only path; adapt tools::card to Tool.input/output/complete and render it from ui/transcript.rs. Keep main's per-session reading bookmarks while remapping live splice anchors and probing the painted last row above the measured floating dock. Tool completion timestamps are additive serde-default metadata; server timestamp strings remain available for reading anchors. Enabling GPUI Component 0.5.1 tree-sitter-languages requires Cargo.lock cc 1.2.67 instead of main's 1.5.1 due to the sequel grammar constraint.

## Impact
Taking either side wholesale loses native features. Repo-bound recon/witness MCP still inspect the original checkout, so worktree source inspection and the full native suite are required.

## Recommendation
Validate both the recovered scroll/tool/timing GPUI regressions and main's response-action, attachment/history, protocol and Rust-host suites when changing this integration.
