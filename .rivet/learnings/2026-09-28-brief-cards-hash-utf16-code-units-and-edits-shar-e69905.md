---
title: Brief cards hash UTF16 code units and edits share the legacy brief lock
date: 2026-09-28
suggested_doc: fleet-manager
related_paths:
  - services/hub-rs/src/services/briefs/document.rs
  - services/hub-rs/src/services/briefs/mod.rs
promoted: false
---

# Brief cards hash UTF16 code units and edits share the legacy brief lock

## Observation
The desktop board hashes each UTF16 code unit through two FNV1a passes (both low and high bytes), so UTF8 byte hashing produces incompatible card IDs for emoji and accented text. Brief parsing and writes must also preserve every original line ending and JavaScript whitespace boundary. Rust brief edits must cooperate on brief.md.lock with the existing Go and TypeScript writers even when service config ownership is isolated: project directories remain shared. Reference timeout/stale policy is3s/15s, with archive written before source removal; the Rust lease also verifies its unique diagnostic token before writing and releasing.
