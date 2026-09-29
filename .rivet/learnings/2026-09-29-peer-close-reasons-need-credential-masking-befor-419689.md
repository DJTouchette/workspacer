---
title: Peer close reasons need credential masking before diagnostic formatting
date: 2026-09-29
promoted: false
---

# Peer close reasons need credential masking before diagnostic formatting

## Observation
Rust Client generic handshake errors and Authorization headers avoid the old query-token dial leak, but a pre-hello WebSocket Close reason remained peer-controlled text and flowed into plugin-dev error diagnostics. Mask credential query values before storing DisconnectReason, so Display, derived Debug and anyhow formatting see only sanitized text while the typed4001 pause code remains intact. Deliberate local pairing URLs are separate outputs and must retain their tokens. The retained Go query cases/idempotence/multiple-match regression passed against the exact Rust helper; real pre-hello close delivery is queued for validation.
