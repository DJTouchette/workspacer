---
title: Case-variant containment corpus cases are unspellable on Windows and assert a FALSE denial there
date: 2026-09-01
confidence: high
suggested_doc: hub-bus-control-plane
related_paths:
  - contracts/path-containment-cases.json
  - services/hub/cmd/brain/fsguard.go
  - services/hub/cmd/brain/pathmatch_windows.go
  - services/hub/cmd/brain/fsguard_test.go
  - services/hub/internal/bus/policy_test.go
  - apps/desktop/src/main/lib/pathConfinement.test.ts
promoted: false
---

# Case-variant containment corpus cases are unspellable on Windows and assert a FALSE denial there

## Observation
Two contracts/path-containment-cases.json cases — "a sibling differing from the root only in case is not inside it" and "a case-variant spelling of a store carve-out is NOT exempted" — build their `tree` from two spellings of ONE name (root/proj + root/PROJ; config/workspacer/library + <configDir>/LIBRARY). On NTFS those are a single directory, so the case's tree collapses and its target IS the granted root's own file. Once cmd/brain started comparing canonical paths with kernel32 CompareStringOrdinal(bIgnoreCase) (commit 0bac5799), those two cases failed on windows-latest — not because the ordinal guard widened authorization, but because a byte-exact deny of the root's own file is a false refusal. The same two case names cascade into TestEveryPathBearingBrainMethodIsConfined for every path-bearing method, which is why one fixture defect produced ~30 subtest failures. Separately, TestBriefToolsAcceptWindowsManagerWorkspaceSpellings failed for an unrelated reason that LOOKED like containment: it sent brief.archive both `count` and `keep`, and archiveOldestEntries refuses the pair ("give either count ... or keep ... and not both"), so the assertion never reached the guard and its message read as a rejection.

## Impact
The corpus is the cross-language contract for three copies of the guard. cmd/brain now folds case on Windows; internal/bus/policy.go and apps/desktop/src/main/lib/pathConfinement.ts are still byte-exact there, so they will refuse a Windows path that is genuinely inside the root they were given. The brain is the copy that answers brief.*/fs.* under the default catalog delegation, so the Fleet Manager defect is fixed, but the other two are a live false-denial divergence. Bus containment only applies to plugin tokens carrying fsRoots (conn.authorize returns early for trusted and scoped user tokens), so the MCP-facade brief_* path is brain-only.

## Recommendation
A corpus case whose target differs from an in-root path only by the case of a directory component must carry the `caseSensitiveOnly` flag; all three loaders skip it on Windows. Do NOT "fix" such a failure by reverting the ordinal comparison. When adding a Windows containment test, cover BOTH polarities: a case-variant allow (kills a byte-exact regression) and a sibling whose path is the root plus more letters, e.g. <root>Other (kills dropping the separator containsPath appends to a non-volume root) — folding the comparison is what makes that widening reachable without a "..".
