---
title: Windows file watcher must freeze Go FileInfo identity before replacement
date: 2026-09-13
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - services/hub/cmd/brain/filewatch.go
  - services/hub/cmd/brain/filewatch_test.go
promoted: false
---

# Windows file watcher must freeze Go FileInfo identity before replacement

## Observation
Real Windows CI emitted eventType=change for an atomic replacement. The test fixture already canonicalizes TempDir with filepath.EvalSymlinks, ruling out the proposed short-path expectation mismatch. Go1.25 os.Stat's Windows fast path caches only attributes and pathname; os.SameFile later calls loadFileId and opens that pathname. Storing a lazy old FileInfo until after replacement makes old and new IDs both describe the replacement. filewatch.go now eagerly resolves identity with os.SameFile(info,info) at sampling, including missing→created samples, and refuses unknown identity instead of treating it unchanged.
