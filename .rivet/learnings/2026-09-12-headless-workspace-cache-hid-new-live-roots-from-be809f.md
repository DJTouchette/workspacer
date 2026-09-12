---
title: Headless workspace cache hid new live roots from immediate browser file operations
date: 2026-09-12
confidence: high
related_paths:
  - services/hub/cmd/brain/fsguard.go
  - services/hub/cmd/brain/live_workspace_test.go
promoted: false
---

# Headless workspace cache hid new live roots from immediate browser file operations

## Observation
The scratch image browser could see a newly hooked session but fs.write still refused its cwd because agentCwds returned a global two-second cached pre-session root list. The live registry already has in-memory snapshots, so it now bypasses the HTTP-only cwd cache whenever r.store exists. New workspace grants and ended-session revocations apply immediately; direct daemon-only registries retain their bounded cache.
