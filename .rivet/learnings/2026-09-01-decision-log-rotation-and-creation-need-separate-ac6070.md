---
title: Decision-log rotation and creation need separate privacy guards
date: 2026-09-01
author: Codex
confidence: high
suggested_doc: limit-aware-routing
related_paths:
  - services/hub/internal/routing/decisionlog.go
  - services/hub/internal/routing/decisionlog_private_windows.go
  - services/hub/internal/routing/decisionlog_unix_test.go
  - services/hub/internal/routing/decisionlog_windows_test.go
promoted: false
---

# Decision-log rotation and creation need separate privacy guards

## Observation
Existing live decision logs need handle-based privacy repair before append and before rename so a loose file cannot become the retained .1 generation. Newly created Windows logs additionally need their protected owner/SYSTEM/Administrators DACL supplied to CreateFile through SECURITY_ATTRIBUTES; OPEN_ALWAYS followed by SetSecurityInfo leaves a creation-time inherited-ACL window.

## Impact
Removing the pre-rename repair exposes rotated audit bytes; relying only on post-open repair briefly exposes newly created audit files through inherited ACLs.

## Recommendation
Keep mutation tests for rotateLocked privacy and use an atomic Windows creation helper while retaining existing-file repair.
