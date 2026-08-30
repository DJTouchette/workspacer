---
title: Host-side git service and review pane flows
tags: [git, review-pane, ipc, hub-bus, diff]
related_paths:
  - "apps/desktop/src/main/services/gitService.ts"
  - "apps/desktop/src/main/services/gitService.test.ts"
  - "apps/desktop/src/renderer/src/lib/gitQueries.ts"
  - "apps/desktop/src/renderer/src/panes/ReviewPane.tsx"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# Host-side git service and review pane flows

## Overview
`apps/desktop/src/main/services/gitService.ts` is a host-side (Electron main) capability that shells out to the `git` binary for the review pane: status, diff, numstat, log (reads) plus stage/unstage/commit/push (writes). It replaced an older HTTP surface in claudemon (now removed) — the agent daemon no longer touches git. It is exposed twice: over preload IPC (`git:*` channels, trusted local desktop) and as hub-bus capabilities (`git.*`, reachable by remote/web/token clients), both backed by the same functions. `apps/desktop/src/renderer/src/lib/gitQueries.ts` is the renderer's thin `GitClient` wrapper over `window.electronAPI`, and `apps/desktop/src/renderer/src/panes/ReviewPane.tsx` is the sole UI consumer.

## Key modules
- `apps/desktop/src/main/services/gitService.ts` — core implementation: `runGit` (execFile wrapper), `workRoot` (resolves cwd via `rev-parse --show-toplevel`), `status`/`diff`/`numstat`/`log` (reads), `stage`/`unstage`/`commit`/`push` (writes via shared `action` helper), parsers (`parsePorcelain`, `parseNumstat`, `parseNumstatPath`, `parseBranchHeader`, `parseLog`), and `formatGitActionError` (maps raw git stderr to actionable messages).
- `apps/desktop/src/main/services/gitService.test.ts` — unit tests for the parsers/formatter (ported from the old Rust test suite).
- `apps/desktop/src/main/shared/ipcChannels.ts` — defines `GIT_STATUS`/`GIT_LOG`/`GIT_DIFF`/`GIT_NUMSTAT`/`GIT_STAGE`/`GIT_UNSTAGE`/`GIT_COMMIT`/`GIT_PUSH` (`git:status` etc.) for the preload/IPC path.
- `apps/desktop/src/main/services/hubCapabilities.ts` — registers `git.status`/`git.log`/`git.diff`/`git.numstat`/`git.stage`/`git.unstage`/`git.commit`/`git.push` (lines ~839-910) as hub-bus capabilities; every handler calls `guardGitCwd` → `assertPathAllowed(cap, cwd, workspaceRoots())` before touching `gitService`, confining a remote caller's `cwd` to live agent cwds + the config dir (see the confinement comment inline, and `assertPathAllowed`).
- `apps/desktop/src/main/preload.ts` — exposes the `git:*` IPC channels as `window.electronAPI.gitStatus/gitDiff/gitNumstat/gitStage/gitUnstage/gitCommit/gitPush`.
- `apps/desktop/src/renderer/src/lib/gitQueries.ts` — renderer `GitClient` class, plus `isUnmergedStatus` (porcelain XY-code conflict detector shared with ReviewPane).
- `apps/desktop/src/renderer/tests/gitQueries.test.ts` — tests `isUnmergedStatus` against the 7 unmerged XY combos (`DD`,`AU`,`UD`,`UA`,`DU`,`AA`,`UU`).
- `apps/desktop/src/renderer/src/panes/ReviewPane.tsx` — the review UI: file tree (conflicts/staged/unstaged/untracked), diff viewer, commit/push actions, error banner, and a "review complete" success banner.

## Failure modes
- `runGit` never rejects on a non-zero git exit (that's a normal read/write failure the caller interprets); it only rejects when `git` itself is missing (`ENOENT` → `"could not run git (is it installed and on PATH?)"`).
- Every op calls `rootOrThrow(cwd)` → `workRoot(cwd)` (`rev-parse --show-toplevel`); if `cwd` isn't inside a work tree, it throws `"cwd is not inside a git work tree"` before any other git command runs.
- Reads (`status`, `diff`, `numstat`) throw `res.stderr.trim() || '<cmd> failed'` verbatim on failure. `log` is lenient: a non-zero exit (e.g. empty repo, no commits yet) yields `[]` rather than throwing.
- Mutating actions go through `action()`, which joins stderr+stdout and passes it through `formatGitActionError` before throwing — this rewrites raw git stderr into actionable copy for merge conflicts, nothing-staged, no-upstream, and rejected-push (non-fast-forward) cases; anything else passes through as the trimmed raw message.
- `diff(..., untracked=true)` treats "produced output" as success rather than exit code, because `git diff --no-index` exits 1 when files differ (the expected case); it also explicitly rejects a path ending in `/`/`\` to avoid git misinterpreting a directory as `dir/null`.
- In `apps/desktop/src/renderer/src/panes/ReviewPane.tsx`, `refresh()` treats `numstat` failures as decoration-only (`.catch(() => [])`) so a numstat hiccup doesn't blank the file tree, but a `status()` failure clears `status` and surfaces `error` in the left-panel banner (dismissable).
- `runAction` (commit/push/stage/unstage) catches and sets `error` from `err.message`, leaving the action buttons `busy`-gated until settled; on success it re-runs `refresh()` and derives a `ReviewNotice` (commit/push banner text) from the *post-action* `GitStatus` (remaining files, ahead count, upstream).

## Gotchas
- **workRoot invariant**: `git status`/`diff --numstat` emit repo-root-relative paths, but `git diff`/`git add` interpret pathspecs relative to the current directory. Every command runs from `workRoot(cwd)`, not `cwd` itself — breaking this silently makes diffs/adds for files outside the immediate subdir come back empty/no-op. Don't add a new git call that uses `cwd` directly.
- **256 MB `MAX_BUFFER`**: whole-tree diffs can be large; the renderer additionally gates *rendering* past `LARGE_DIFF_CHARS` (1.5M chars, in `apps/desktop/src/renderer/src/panes/ReviewPane.tsx`) behind an explicit "Render anyway" click, but the full text is always fetched first — a diff between 1.5MB and 256MB is fully transferred over IPC/hub-bus before the size gate applies.
- **Hub-bus is the remote-reachable git surface now**: since git left the daemon, `apps/desktop/src/main/services/hubCapabilities.ts`'s `guardGitCwd`/`assertPathAllowed` is the only confinement stopping a bus token holder from reading/committing/pushing an arbitrary repo; the desktop IPC path has no such guard (trusted local user). Any new `git.*` capability must call `guardGitCwd` first.
- **Twin type/interface duplication**: `FileStatus`/`GitStatus`/`NumstatEntry` are declared independently in both `apps/desktop/src/main/services/gitService.ts` (host) and `apps/desktop/src/renderer/src/lib/gitQueries.ts` (renderer) — the renderer's `GitStatus.upstream/ahead/behind` are optional (`?`) specifically to tolerate an older host over the hub bus that predates those fields; keep both definitions and that optionality in sync when changing the shape.
- **Rename parsing has two independent formats** that must stay consistent: porcelain `-z` renames put the source path as a *separate* NUL token (`parsePorcelain`), while numstat renames appear inline as `old => new` or brace form `prefix/{old => new}/suffix` (`parseNumstatPath`) — a bug fix in one format won't fix the other.
- `isUnmergedStatus` (in `apps/desktop/src/renderer/src/lib/gitQueries.ts`, reused by ReviewPane to bucket files into the Conflicts section) hardcodes the 7 XY unmerged combos (`DD,AU,UD,UA,DU,AA,UU`) plus a fallback for either code being `U` — if git ever adds a new conflict marker combo this needs a matching update.
- `apps/desktop/src/renderer/src/panes/ReviewPane.tsx`'s "Push" button is greyed out only when `status.upstream != null && ahead === 0`; with no upstream (or an older host omitting ahead/behind) it stays enabled so a real failure reason surfaces via `formatGitActionError` instead of a false "nothing to push".

## Hand-authored notes (2026-08-24) — `git.*` now has TWO providers, and the untracked-diff leg needs BOTH root sets

The read-only half of `git.*` (status/log/diff/numstat) is now provided by BOTH
`apps/desktop/src/main/services/hubCapabilities.ts` and
`services/hub/cmd/brain/git.go`. Two things are non-obvious:

1. **The hub router is single-owner per METHOD and first-registration-wins**, so
   in the "adopted hub" configuration the brain wins the four reads while the
   desktop keeps commitDiff/commitNumstat/stage/unstage/commit/push — no method is
   owned twice, and the split is stable rather than a race.
   `services/hub/cmd/brain/delegation_guard_test.go`'s `declaredOverlap` is where
   that has to be written down or the package fails. Anyone adding a fifth
   brain-provided git method must add a `declaredOverlap` entry too.
2. **`git.diff{untracked}` is an arbitrary-file reader unless its `path` is held
   to TWO root sets**: the DERIVED work-tree root (which comes out of
   `rev-parse --show-toplevel` AFTER the cwd guard and is never itself
   allow-listed) **AND** the ordinary workspace roots. Holding it to the repo
   alone lets an agent cwd of `<repo>/frontend` read `<repo>/backend/.env`,
   because `git diff --no-index -- /dev/null <path>` reads the operand as a
   filesystem path and renders any readable file as an all-added diff.

**Anyone "simplifying" `anchorGitPathspec` to resolve against the caller's cwd
reopens a documented arbitrary-file read on an internet-facing node.** Before
touching either provider's git block, read the `anchorGitPathspec` comment in
`services/hub/cmd/brain/git.go` and run `go test ./cmd/brain -run TestGitDiff`. The mutant
that drops the extra `workspaceRoots` assertion is caught by
`TestGitDiffUntrackedCannotEscapeTheAllowedRoots/a_sibling_subtree_of_the_agent_cwd`;
**if that test ever needs weakening, the leg is wrong, not the test.** Note
`git.*` is operator-tier only (`authtoken` `Scope.Methods()`: view/triage carry
zero git entries, operator is `"*"`), so no tier work is involved.

## Hand-authored notes (2026-08-28) — recovering a dirty worktree: search sibling commits BEFORE applying the diff

A stale checkout's branch NAME and its distance from master are not reliable
descriptions of its working patch. A worktree named for a committed stall fix
carried a 24-file working diff that actually implemented **spawn-carried
first-message delivery**; its HEAD (`da5e8710`) and the landed feature commit
(`827d6d33`) were SIBLING commits with the same parent. At merge `55c84e1a`, 19
of the 24 dirty files matched the stale working files byte-for-byte, and the
other five retained the behaviour while adding federated version-skew fallback,
bounded failure tracking, and pending-slot ownership hardening — so current
master had no stale-only behaviour left to port at all.

**Blindly applying the old diff would have duplicated an existing feature and
regressed current safeguards.** The procedure: inspect the dirty PATHS and
vocabulary first, use `git log -S` or a commit-message search to find sibling
implementations, then compare the stale working file hashes against the landing
merge before attempting a port. **Treat an already-landed superset as a
successful recovery and commit only the audit record.**
