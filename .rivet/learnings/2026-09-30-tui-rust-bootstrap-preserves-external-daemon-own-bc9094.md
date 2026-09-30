---
title: TUI Rust bootstrap preserves external daemon ownership
date: 2026-09-30
promoted: false
---

# TUI Rust bootstrap preserves external daemon ownership

## Observation
apps/tui/src/daemons.rs launches workspacer-rust serve only for a missing loopback bus with no existing daemon owner. The guard closes child stdin and waits before kill fallback; direct claudemon bootstrap remains a separate fallback. Updated tui-client context to remove obsolete Go brain supervision instructions. This is source inspection, not a new live TUI-to-Rust integration receipt.
