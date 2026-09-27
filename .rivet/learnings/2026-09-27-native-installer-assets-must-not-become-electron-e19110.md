---
title: Native installer assets must not become Electron downloads
date: 2026-09-27
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - landing/index.html
  - apps/native/scripts/windows-payload.mjs
  - services/hub/cmd/brain/desktophost.go
promoted: false
---

# Native installer assets must not become Electron downloads

## Observation
landing/index.html previously selected the first release asset matching Setup.*.exe, which would also match Workspacer-Native-Setup after native publishing is added. Its Electron selector now excludes native filenames. Native local packaging also needs desktop-host.cjs plus a private Node runtime in addition to the four Go binaries; Windows brain resolves a sibling node.exe independently of its bundle override and cwd.
