---
title: Drain sidecar pipes during graceful termination
date: 2026-09-29
promoted: false
---

# Drain sidecar pipes during graceful termination

## Observation
NativeChild.stop previously waited for SIGTERM completion before draining stdout/stderr. A cooperative handler flushing two MiB blocked on its full pipe and was killed after five seconds. Keep bounded nonblocking pipe drains inside the grace wait. The self-executable supervisor_shutdown fixture reproduces the failure without shell/provider dependencies; log framing remains intentionally bounded to64KiB with CR normalization, unlike Go unlimited lines.
