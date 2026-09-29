---
title: Negotiated relay cancellation must revoke pending admission without abandoning accepted engine effects
date: 2026-09-28
author: codex
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - services/hub-rs/src/client.rs
  - services/hub-rs/src/runtime.rs
  - services/hub-rs/src/provider_relay/mod.rs
promoted: false
---

# Negotiated relay cancellation must revoke pending admission without abandoning accepted engine effects

## Observation
Dropping Client.call futures previously removed only the local oneshot waiter; central pending calls and delegated relay connections could remain active. Rust now emits cancel only for embedded callers or explicit identity-v1 remote negotiation. Core resolves original caller connection plus correlation, removes its pending launch permit, and sends provider numeric-ID cancel only to wantsCallerContext providers. Relay aborts the matching per-link attempt waiter while retaining sibling calls. Link loss explicitly closes cached delegated clients even if active tasks hold Arc references. Local owned handlers finish accepted-effect bookkeeping rather than being blindly aborted.

## Impact
A timed-out plugin preparation must not later use a revoked owner permit; canceling a whole cached delegated client would incorrectly cancel sibling requests and terminal leases. Accepted engine effects cannot be assumed undone or safely replayed.

## Recommendation
Keep final CheckLaunch(finish=true) after all awaited preparation. Test broker permit removal separately from slow-hook Lifecycle rejection and accepted-operation survival; preserve old-peer wire behavior until explicit negotiation.
