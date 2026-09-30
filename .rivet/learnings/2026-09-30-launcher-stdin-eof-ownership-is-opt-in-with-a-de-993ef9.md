---
title: Launcher stdin EOF ownership is opt-in with a declared parent
date: 2026-09-30
suggested_doc: modules/workspacer-serve-cli.md
related_paths:
  - services/hub-rs/src/cli/parent.rs
promoted: false
---

# Launcher stdin EOF ownership is opt-in with a declared parent

## Observation
Rust cli/parent.rs leaves parent monitoring pending unless WORKSPACER_PARENT_PID is nonempty. Once opted in it monitors both parent death and stdin EOF. The old workspacer-serve-cli context described retired Go sibling supervision and unconditional child planning.

## Impact
Operational docs must not promise foreground shutdown on stdin EOF or require retired private Node companion binaries.

## Recommendation
Document full in-process Rust ownership, explicit hub-only borrowing, and parent opt-in separately from plugin sidecar supervision.
