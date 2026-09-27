---
title: GPUI native backend must join in synchronous app termination callback
date: 2026-09-27
promoted: false
---

# GPUI native backend must join in synchronous app termination callback

## Observation
GPUI macOS platform quit uses NSApplication termination, so Application::run is not guaranteed to return for Rust cleanup. GPUI invokes on_app_quit observers synchronously before applying a 100ms deadline to their returned futures. Native backend owner is shared through Rc/RefCell; the committed quit observer synchronously takes and joins the host then returns a ready future. Post-run fallback uses the same optional owner and preserves the result, preventing double teardown. Normal rendering remains asynchronous. GPUI test exercises cx.shutdown and verifies backend stopped.
