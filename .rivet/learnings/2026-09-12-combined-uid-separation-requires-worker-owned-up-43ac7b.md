---
title: Combined UID separation requires worker-owned uploads and authenticated network callbacks
date: 2026-09-12
confidence: high
suggested_doc: fly-node-deploy
related_paths:
  - services/hub/internal/uploads/store.go
  - services/hub/cmd/hub/upload.go
  - services/hub/internal/bus/desktop.go
  - deploy/fly/combined/network-admin.py
  - deploy/fly/combined/upgrade-parity-supervisor.py
promoted: false
---

# Combined UID separation requires worker-owned uploads and authenticated network callbacks

## Observation
The hub's old files.upload wrote 0600 files under its own UID, unreadable by combined-node agents running as UID 10001. Isolated/full headless uploads now forward via hub self-client to owner-gated files.receiveUpload, using the same limits/extensions in shared uploads.Store and a per-UID temp directory. Failures never fall back to hub-owned bytes. Separately, socket UID alone is insufficient for the root network helper because plugin sidecars can share hub UID: the helper now also requires a random bearer kept in the protected hub config directory, passed only to the root helper's environment (hub receives a file path).
