---
title: Rust remote dispatch separates worker leases from origin receipts
date: 2026-09-28
promoted: false
---

# Rust remote dispatch separates worker leases from origin receipts

## Observation
Go brain and the private Node origin registry both used remote-dispatches.json with incompatible array schemas (worker id/session/lease versus origin dispatchId/ownerSessionId/deliveringSeq). A co-located Rust runtime uses remote-dispatch-worker.json and remote-dispatch-origin.json, migrates only a homogeneous recognized legacy role, and leaves the original file untouched. Worker claim and origin deliveringSeq must persist before effect: a missing remote journal or missing acknowledgement is an unknown outcome, never authorization to replay spawn or resend a wake. The receiver binds a lease to the authenticated federation credential fingerprint, not a claimed peer name; only the trusted federation envelope supplies the callback peer. Remote paths remain opaque on the origin and are checked only at execution.
