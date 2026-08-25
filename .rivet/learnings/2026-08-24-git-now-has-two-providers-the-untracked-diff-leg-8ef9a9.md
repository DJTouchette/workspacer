---
title: git.* now has TWO providers; the untracked-diff leg needs both root sets
date: 2026-08-24
confidence: high
suggested_doc: git-review
promoted: false
---

# git.* now has TWO providers; the untracked-diff leg needs both root sets

## Observation
The read-only half of git.* (status/log/diff/numstat) is now provided by BOTH apps/desktop/src/main/services/hubCapabilities.ts and services/hub/cmd/brain/git.go. Two things are non-obvious. (1) The hub router is single-owner per METHOD and first-registration-wins, so in the "adopted hub" configuration the brain wins the four reads while the desktop keeps commitDiff/commitNumstat/stage/unstage/commit/push — no method is owned twice, and the split is stable rather than a race. delegation_guard_test.go's declaredOverlap is where that has to be written down or the package fails. (2) git.diff{untracked} is an arbitrary-file reader unless its `path` is held to TWO root sets: the DERIVED work-tree root (which comes out of `rev-parse --show-toplevel` AFTER the cwd guard and is never itself allow-listed) AND the ordinary workspace roots. Holding it to the repo alone lets an agent cwd of <repo>/frontend read <repo>/backend/.env, because `git diff --no-index -- /dev/null <path>` reads the operand as a filesystem path and renders any readable file as an all-added diff.</observation>
<parameter name="impact">Anyone adding a git.* method to the brain, or "simplifying" anchorGitPathspec to resolve against the caller's cwd, reopens a documented arbitrary-file read on an internet-facing node. Anyone adding a fifth brain-provided git method must also add a declaredOverlap entry or cmd/brain fails to build green.</impact>
<parameter name="recommendation">Before touching either provider's git block, read the anchorGitPathspec comment in cmd/brain/git.go and run `go test ./cmd/brain -run TestGitDiff`. The mutant that drops the extra workspaceRoots assertion is caught by TestGitDiffUntrackedCannotEscapeTheAllowedRoots/a_sibling_subtree_of_the_agent_cwd; if that test ever needs weakening, the leg is wrong, not the test. Note git.* is operator-tier only (authtoken Scope.Methods(): view/triage carry zero git entries, operator is "*"), so no tier work is involved.</recommendation>
<parameter name="related_paths">["services/hub/cmd/brain/git.go", "services/hub/cmd/brain/gitconfinement_test.go", "apps/desktop/src/main/services/hubCapabilities.ts", "services/hub/cmd/brain/delegation_guard_test.go"]
