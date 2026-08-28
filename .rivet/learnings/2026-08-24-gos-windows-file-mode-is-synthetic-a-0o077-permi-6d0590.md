---
title: Go's Windows file mode is synthetic — a 0o077 permission check is a CONSTANT there, not a check
date: 2026-08-24
confidence: high
suggested_doc: hub-federation
related_paths:
  - services/hub/internal/nodes/exposure*.go
  - services/hub/cmd/hub/nodes.go
  - services/hub/cmd/hub/upload_test.go
  - services/hub/cmd/brain/unpinnedguards_test.go
promoted: false
---

# Go's Windows file mode is synthetic — a 0o077 permission check is a CONSTANT there, not a check

## Observation
os.FileInfo.Mode().Perm() on Windows is synthesised from ONE attribute bit (os/types_windows.go: `if FILE_ATTRIBUTE_READONLY { m |= 0444 } else { m |= 0666 }`), so Perm() is 0666 for every writable file and 0444 for every read-only one — `Perm()&0o077 != 0` is TRUE in both cases, for every file that exists. os.Chmod is the same story inverted: on Windows it only toggles FILE_ATTRIBUTE_READONLY (syscall/syscall_windows.go), so `os.Chmod(p, 0o644)` grants no principal anything. This made internal/nodes.FileLooksExposed (the on-disk guard for the Fly API token in nodes.json) report "readable beyond its owner" on every Windows start regardless of the file's real ACL, and CI caught it as a red TestFileLooksExposedNoticesLoosePermissions on the containment-windows job. The test was a true positive; the check was the broken half.

## Impact
A permission check written the Unix way is not merely imprecise on Windows, it carries zero information — and a warning that fires unconditionally is worse than no warning, because the user learns to scroll past it. The same trap is waiting for peers.json, tokens.json, jobs.json and remote-token, all of which are 0600 BY CONVENTION with no check at all today.

## Recommendation
Two different situations, two different answers, and the repo now has a precedent for each. (1) Asserting the mode WE WROTE (a write we control) → skip on Windows: `if onWindows { t.Skip("POSIX mode bits") }` (cmd/brain/unpinnedguards_test.go) or `runtime.GOOS != "windows" &&` (cmd/hub/upload_test.go). (2) A shipped check that renders a VERDICT about who can read a file → do NOT skip; the skip leaves a live check that lies. Use internal/nodes.FileExposure (exposure.go + exposure_unix.go + exposure_windows.go): three-valued (ExposureLoose / ExposureOwnerOnly / ExposureUnknown, zero value Unknown so a forgotten case never reads as safe), with a real DACL walk on Windows via golang.org/x/sys/windows (already a direct dep) — GetNamedSecurityInfo → DACL() → GetAce, read grant to a well-known everyone-ish SID = loose, NULL DACL = loose (grants EVERYONE full control), domain group / unparseable ACE = unknown. Lift that file if a second 0600 credential file ever grows a check.
