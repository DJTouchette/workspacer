---
title: Rust manager handoff needs a capture gate and reply-fenced acknowledgements
date: 2026-09-28
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/src/services/manager_replacements.rs
  - services/hub-rs/src/services/manager_replacements/messages.rs
  - services/hub-rs/src/services/manager_replacements/native.rs
promoted: false
---

# Rust manager handoff needs a capture gate and reply-fenced acknowledgements

## Observation
The Node replacement start snapshots in-flight messages and commits its journal without yielding, so ACK completion cannot interleave. Rust now shares MessageTracker.capture_gate between replacement start and send begin/finish, preserving that atomic boundary. Durable transferIntent redirects host inbox captures inside TaskStore transactions, including the gap after task adoption but before worker-transfer ACK. Signature acknowledgements clear saved finishes only when both reply and stopped evidence still match; metadata capture updates prior manager records even on explicit role downgrade.

## Impact
Without these concurrency boundaries, handoff can strand user requests on the retired owner, record already-accepted sends as forever sending, or discard a newer worker completion on an older ACK.

## Recommendation
Route all manager-bound sends through the shared tracker; never hold wake-actor locks while calling that sender. Retain task/worker transfer intent for roll-forward recovery, require real viewer bindings for kickoff, and use record_signature_for_finish for terminal wake ACKs.
