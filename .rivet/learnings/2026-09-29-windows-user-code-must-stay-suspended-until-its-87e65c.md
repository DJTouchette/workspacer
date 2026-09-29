---
title: Windows user code must stay suspended until its child job is installed
date: 2026-09-29
promoted: false
---

# Windows user code must stay suspended until its child job is installed

## Observation
Stable Rust1.98 spawn_with_attributes remains nightly-only, so the shared Windows helper now uses documented CREATE_SUSPENDED, AssignProcessToJobObject and ResumeThread. It opens the sole initial thread, then checks its owner PID plus original process-handle liveness AFTER opening the thread handle; a recycled numeric PID/TID cannot retarget the handle. A suspend count other than1 refuses launch, and failed assignment remains fail-closed. Standard/Tokio commands, plugin install/supervisor/Codex probe, and remote readiness all share this owner; ConPTY opts into a minimal MIT portable-pty0.8.1 patch exposing creation flags default0. Host never joins a child job. Exact Windows source probe passes, but immediate-fork std/Tokio/ConPTY tests require actualWindows CI before claiming runtime completion. Readiness also now uses the common owned capture instead of its old post-wait numeric Unix group signal.
