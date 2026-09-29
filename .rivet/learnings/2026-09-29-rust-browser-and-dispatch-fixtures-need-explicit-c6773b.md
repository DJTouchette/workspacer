---
title: Rust browser and dispatch fixtures need explicit build and transport boundaries
date: 2026-09-29
suggested_doc: mcp-tool-facade
related_paths:
  - services/hub-rs/examples/hub_contract_fixture.rs
  - apps/desktop/tests/e2e/fixtures/rustHub.ts
promoted: false
---

# Rust browser and dispatch fixtures need explicit build and transport boundaries

## Observation
The Go-free app/mobile/dispatch fixtures compile feature-gated hub_contract_fixture through Cargo JSON compiler-artifact output. Build children must preserve CARGO_INCREMENTAL and CARGO_TARGET_DIR while runtime children use scratch HOME/XDG. A cold build exceeds browser 30-second hooks. Rust Streamable HTTP is stateless and may omit MCP-Session-Id; initialization tracking must not require a session ID. Full dispatch tests exposed null-versus-omitted afterDispatchId in first workflow steps across Rust MCP and retained Electron workflow services.

## Impact
Ignoring these distinctions either fills the target volume, fails setup before behavioral assertions, or rejects valid first workflow dispatches.

## Recommendation
Keep opt-in test-support controls outside default builds, run dispatch-chain plus app/mobile Playwright suites, and preserve optional-field omission across language boundaries.
