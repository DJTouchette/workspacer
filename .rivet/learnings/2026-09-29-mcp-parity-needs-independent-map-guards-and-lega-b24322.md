---
title: MCP parity needs independent map guards and legacy receipt behavior
date: 2026-09-29
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub-rs/src/mcp/**
  - services/hub-rs/tests/mcp/**
promoted: false
---

# MCP parity needs independent map guards and legacy receipt behavior

## Observation
Remaining Go facade assertions exposed three gaps: Rust lacked the independent wholesale-map guard behind save_config schema validation, task-reference empty taskId/cwd checks were absent, and legacy bare JSON session IDs could not receive first-message fallback. Bare model overrides also emitted contextWindow:null rather than the original omitted companion. Real HTTP/broker parity fixtures now exercise these cases. Frozen Go catalog/help inputs remain unchanged; an authored Rust presentation overlay corrects obsolete headless-unavailable claims without changing schema constraints.

## Impact
Catalog equality alone did not prove refusal-before-provider behavior or composed dispatch compatibility.

## Recommendation
Keep independent config guard tests and real provider call logs for facade migrations; preserve explicit uncertain-delivery no-replay behavior.
