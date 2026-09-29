---
title: PTY EOF must follow the authoritative terminal state transition
date: 2026-09-29
promoted: false
---

# PTY EOF must follow the authoritative terminal state transition

## Observation
macOS CI exposed a race in SessionStore teardown: release_spawn_plumbing removed the byte broadcaster before release_spawn marked the row stopped or drop_pending_spawn removed it. The Rust terminal forwarder receives Closed, reads canonical state once, and could see a live row, losing pty.exit forever. Canonical state now transitions first while existing generation checks remain; a deterministic regression holds the byte-sender map entry to prove stopped/absent state precedes its release in both paths. Separately manager checkpoint verification must use the shared Windows canonical DOS path helper, not std canonicalize verbatim prefixes fed back into the strict plain-path grammar.
