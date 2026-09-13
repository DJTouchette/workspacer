---
title: Windows artifact storage requires native relative handles, not Node nofollow flags
date: 2026-09-13
confidence: high
suggested_doc: git-review
related_paths:
  - apps/desktop/src/main/services/intentWindowsFiles*.ts
  - apps/desktop/src/main/services/intentArtifactFiles.ts
  - .github/workflows/ci.yml
promoted: false
---

# Windows artifact storage requires native relative handles, not Node nofollow flags

## Observation
Node's Linux O_DIRECTORY/O_NOFOLLOW + /proc/self/fd strategy cannot secure Windows artifact paths. intentWindowsFiles embeds C# compiled by built-in Windows PowerShell5.1; it opens drive/UNC root handles, then NtCreateFile RootDirectory-relative components/leaves with FILE_OPEN_REPARSE_POINT, rejects reparse points and hardlinked files, and retains parent handles denying WRITE/DELETE sharing. Byte operations are structured and stay inside that native lease. Existing context replacement uses one exclusive handle for digest check+write; partial failures retain the existing unknown receipt protocol. Shared knowledge listings batch Windows reads by category. Initial verification is C#5 syntax compilation on Linux, with actual Windows semantics exercised only by the intent-windows-files CI lane.

## Recommendation
Do not replace native relative opens with realpath-plus-Node-path operations, expose leafPath across helper lifetime, or allow device/ADS/reserved-name aliases. Keep Windows runtime lane mandatory for these surfaces and clearly distinguish local compile checks from actual Windows execution.
