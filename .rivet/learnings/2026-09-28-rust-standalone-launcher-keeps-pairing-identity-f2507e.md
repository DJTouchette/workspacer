---
title: Rust standalone launcher keeps pairing identity and process ownership in one runtime
date: 2026-09-28
promoted: false
---

# Rust standalone launcher keeps pairing identity and process ownership in one runtime

## Observation
The experimental workspacer-rust CLI launches Backend directly (Rust claudemon engine plus hub/MCP), never resolving Go or Node siblings. It shares config/remote-token, refuses minting after suspected identity loss, serializes first-run token creation with an OS advisory lock, and still requires explicit database paths for alternate API/hook ports. Hook initialization gained a quiet Rust API so --json readiness stdout remains machine-readable. Plugin dev rebuilds in the real source directory and reloads through host HTTP, preserving relative imports and the source token/settings instead of relocating author builds.
