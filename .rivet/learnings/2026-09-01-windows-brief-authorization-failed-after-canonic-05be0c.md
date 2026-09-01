---
title: Windows brief authorization failed after canonicalization at byte-exact root comparison
date: 2026-09-01
confidence: high
suggested_doc: fleet-manager
related_paths:
  - services/hub/cmd/brain/fsguard.go
  - services/hub/cmd/brain/pathmatch_windows.go
  - services/hub/cmd/brain/brief_windows_test.go
promoted: false
---

# Windows brief authorization failed after canonicalization at byte-exact root comparison

## Observation
The brain canonicalizer accepts both Windows slash styles but preserves drive and component casing; containsPath then compared canonical roots byte-for-byte. A Fleet Manager's path spelling could therefore miss its own live cwd or a config-store root on Windows even though both spellings open the same directory. All brief tools share this predicate through workspaceRoots.

## Impact
The shared denial made brief.append, brief.check, and brief.archive fail before their per-tool derived-path guards ran.

## Recommendation
Keep root authorization byte-exact on non-Windows hosts and use CompareStringOrdinal(ignore-case) only for Windows canonical path equality/prefix checks. Pin the manager-only root flow plus an outside sibling denial in a Windows test.
