---
title: Terminal byte precedence must not bypass typed text validation
date: 2026-09-29
confidence: high
suggested_doc: headless-desktop-services
related_paths:
  - services/hub-rs/src/services/terminals.rs
  - services/hub-rs/tests/terminals.rs
promoted: false
---

# Terminal byte precedence must not bypass typed text validation

## Observation
Actual registered sessions.terminalInput accepted bytesB64 AQI= with data42 because Rust only validated data in the text branch; Go decoded the entire typed struct first and rejected it. New real PTY regression reproduced that acceptance, then the narrow fix validates data before selecting bytes/text while preserving nonempty-byte precedence and newline=false. Additional tests require the complete visible-terminal event payload and stopped-bus refusal rather than silent success; visible open remains an event request, not proof a UI displayed a pane.

Actual registered root-shape probes additionally demonstrated terminals.open accepted an array as empty parameters. Terminal methods now explicitly require an object or null, preserving the Go null/default visible-open control. Scalar/array probes cover open, input and create; final validation is tracked in brain-terminal-controls.json.
