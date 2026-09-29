---
title: Windows atomic JSON replacement needs bounded sharing retries
date: 2026-09-29
promoted: false
---

# Windows atomic JSON replacement needs bounded sharing retries

## Observation
Actual773 Windows layout disk-observer test kept monotonic readable JSON but exposed transient MoveFileEx AccessDenied during replacement; final live version2401 differed from disk2400. tempfile3.27 uses MoveFileExW(REPLACE_EXISTING) and returns the original NamedTempFile on failure. Retrying only Windows raw5/32/33 retains that same staging file and leaves old target intact; never delete or copy over the target. Keep 8x300 concurrent-writer/read-observer assertion. Primary references: https://learn.microsoft.com/en-us/windows/win32/fileio/moving-and-replacing-files and tempfile/src/file/imp/windows.rs.
