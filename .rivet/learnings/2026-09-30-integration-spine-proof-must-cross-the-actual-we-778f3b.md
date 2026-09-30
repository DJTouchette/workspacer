---
title: Integration spine proof must cross the actual WebSocket adapter
date: 2026-09-30
confidence: high
suggested_doc: hub-plugin-system
related_paths:
  - services/hub-rs/tests/integration_spine.rs
promoted: false
---

# Integration spine proof must cross the actual WebSocket adapter

## Observation
The retained editor_sandbox test already asserts ambient filesystem access outside agent cwd; its legacy filename is not a sandbox policy. Existing Rust pane-token and fs tests separately proved identity and ambient access, while external SSE tests reached an embedded Client without asserting the final WebSocket envelope. New integration_spine target joins those actual paths and checks broker stamping, rather than treating helper tests as end-to-end evidence.

## Recommendation
Keep enabled plugin ambient access separate from host-only administration, provider namespace admission, UI object containment and browser isolation. Use subscribed-channel handshake before releasing SSE data, not timing sleeps.

## Validation
Both actual WebSocket integration tests passed on Linux in /tmp/workspacer-integration-spine-final.log. Desktop browser policy/root tests91 also passed in /tmp/workspacer-spine-webview-gates.log; this is not a claim of real Chromium navigation testing.
