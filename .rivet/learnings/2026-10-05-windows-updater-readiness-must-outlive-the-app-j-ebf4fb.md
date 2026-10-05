---
title: Windows updater readiness must outlive the app job and dirty prompt
date: 2026-10-05
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/native/src/updates.rs
  - apps/native/src/update_helper.ps1
  - apps/native/tests/windows_update_handoff.rs
  - apps/native/src/ui/file_viewer.rs
promoted: false
---

# Windows updater readiness must outlive the app job and dirty prompt

## Observation
A ready PowerShell process is insufficient if it inherits the app kill-on-close job: refuse breakaway denial rather than retrying in that job. Retain a process+nonce readiness probe, and make Save/Discard for an update recheck it instead of using ordinary CloseWindow. A timer-only retry can arm overlapping helpers before the first app-exit timeout. Per-nonce plans plus a helper mutex prevent overlap; a timed-out installer retains that mutex until it exits. Windows fixture installers must emulate NSIS raw /D remainder parsing, not CRT argv splitting.
