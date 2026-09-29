---
title: Rust workflow metadata reads documented literals without evaluating JavaScript
date: 2026-09-28
suggested_doc: workflow-subagent-watcher
related_paths:
  - Keep metadata interpretation data-only; extend literal fixtures if the provider emits a new literal syntax. Do not hide computed-expression differences behind a full parity claim.
promoted: false
---

# Rust workflow metadata reads documented literals without evaluating JavaScript

## Observation
Legacy workflowWatcher.parseScriptMeta evaluates a captured export const meta object in Node vm with a50ms limit even though its documented producer contract requires a pure literal. The Rust workflow artifact port uses bounded JSON5 data parsing for unquoted keys, single quotes, comments and trailing commas and never executes metadata. Computed/functional JavaScript object members fall back to the filename-derived workflow name; this is an explicit compatibility limitation, not full VM equivalence. Live transcript/journal/final-file and telemetry transitions are compared against a generated fixture running the actual TS watcher with real temporary files.
