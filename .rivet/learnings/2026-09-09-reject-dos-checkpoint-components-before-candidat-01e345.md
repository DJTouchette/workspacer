---
title: Reject DOS checkpoint components before candidate traversal
date: 2026-09-09
confidence: high
suggested_doc: fleet-manager
related_paths:
  - apps/desktop/src/main/services/managerReplacementArtifact.ts
promoted: false
---

# Reject DOS checkpoint components before candidate traversal

## Observation
Microsoft naming-a-file reserves CON/PRN/AUX/NUL and COM/LPT with digits 1-9 or superscripts ¹²³ in every directory, including extension spellings. A drive-qualified checkpoint could pass plainAbsolutePath and reach candidate lstat before rejection. The validator now rejects these components before I/O; CreateFileW documents CONIN$ and CONOUT$ separately as exact console names, so this policy rejects those exact components without inventing DOS extension aliases. Candidate root spellings are also filtered against host-known roots before traversal, using the existing spelling rule only as a rejection filter; filesystem identity remains authoritative.

## Impact
The fake Win32 regression table reproduced 148 failures on ab89bf2b, then passed with zero candidate calls for reserved spellings at three depths and both separator styles. Safe lookalikes, case-variant identity, and ordinary POSIX names remain accepted. Microsoft sources: https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file and https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew#consoles.

## Recommendation
Keep the full documented name table and mocked access spies; run the existing native Windows CI suites without probing real devices or SMB. Linux fixture verification does not establish native Windows execution or the original unavailable remote artifact failure cause. The existing non-atomic ancestor traversal limitation is unchanged.
