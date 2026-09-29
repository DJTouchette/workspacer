---
title: Remote pairing info is public scoped metadata and socket revocation is periodic
date: 2026-09-29
suggested_doc: remote-mobile
related_paths:
  - Do not tighten public information endpoints to make a revocation test pass. Wait for the documented live-connection revalidation interval and separately assert new authentication is refused.
promoted: false
---

# Remote pairing info is public scoped metadata and socket revocation is periodic

## Observation
remote.pairingInfo intentionally answers authorized non-owner callers with their scope and canManageTokens:false; remote.sharingInfo similarly exposes availability without management authority. Mint/list/revoke and Tailscale control remain owner-gated. Rust Core revalidates existing ordinary scoped sockets every5s, so removing a pairing immediately rejects a new connection but a live existing socket closes on that bounded sweep, not synchronously with tokenRevoke. The integration test now verifies both edges instead of mistaking public pairingInfo access or the sweep interval for an owner-gate defect.
