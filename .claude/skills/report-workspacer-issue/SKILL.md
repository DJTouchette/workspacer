---
name: report-workspacer-issue
description: File a Workspacer bug on GitHub, and open a fix PR when the fix is clear, from an in-app Diagnose report or a problem the user hit in Workspacer itself. Use when the user pastes a Diagnose report ("## What I saw … ## Diagnosis"), says "report this", "file an issue/ticket", or asks to fix-and-PR a Workspacer bug. Always shows the draft and asks before posting anything.
---

# Report a Workspacer issue (and fix it when it is clear)

Workspacer is open source: https://github.com/DJTouchette/workspacer. The
native app's **Diagnose** button (an error notice, or a background task's log)
asks the user's own harness to explain the error. When the answer judges the
fault to be Workspacer's own, the app offers **Report issue…** (a pre-filled
GitHub page) and **Copy** (the full report as Markdown). This skill picks up
from that report, or from a description of the problem.

Posting is outward-facing and public. **Never create an issue, comment, push
or open a PR without the user approving the exact text first.**

## 1. Gather

- The report: the user's paste, or ask for it (Diagnose → Copy).
- Versions: `VERSION` at the repo root (or the app's version in the report),
  the OS, the agent CLI and its version if it matters (`claude --version`,
  `codex --version`).
- Reproduction: the shortest steps that show it. Say which steps are
  confirmed and which are guessed.

## 2. Scrub

Remove anything private before drafting: user names and home paths
(`C:\Users\<name>` → `%USERPROFILE%`, `/home/<name>` → `~`), project names
and paths the user did not mean to share, emails, hostnames, tokens and
keys (the hub already masks well-known token shapes; check anyway), and
customer data in log lines. Ask when unsure whether something is private.

## 3. Check for duplicates

```bash
gh issue list --repo DJTouchette/workspacer --state all --search "<key words>" --limit 20
```

If one matches, offer to comment there (with the new evidence) instead of
opening another.

## 4. Draft and confirm

Title: the symptom, specific and short ("Task log stays blank when the
command logs to `$(mktemp)`"). Body:

```markdown
## What happened
## Expected
## Steps to reproduce
## Diagnosis
(the model's reading, labelled as such; it had no tools and may be wrong)
## Environment
Workspacer <version>, <OS>, <harness + version>
```

Show the title and body to the user and wait for approval. Then:

```bash
body=$(mktemp)   # write the approved body here with your file tool first
gh issue create --repo DJTouchette/workspacer --title "<title>" --body-file "$body" --label bug
```

`gh` needs `gh auth login` once; if it is not signed in, give the user the
pre-filled link instead (`https://github.com/DJTouchette/workspacer/issues/new?title=…&body=…`).

## 5. Fix and open a PR (only when the cause is clear)

Only in a checkout of this repository, and only when the user asks for it.

1. Read the repo's `CLAUDE.md` first; follow its Rivet steps (context docs,
   `recon.related`, `witness.select`) and the native design rules.
2. Branch from the default branch: `git switch -c fix/<short-name>`.
3. Make the smallest change that fixes the cause, with a test that fails
   before it and passes after. Run what `witness.select` names, plus
   `cargo clippy -- -D warnings` for touched Rust crates.
4. Commit only what the fix needs; reference the issue (`Fixes #<n>`).
5. Show the user the diff summary and test results, and ask before pushing.
   Contributors without write access push to their fork
   (`gh repo fork --remote`), then:

   ```bash
   gh pr create --repo DJTouchette/workspacer --title "<title>" --body-file "$body"
   ```

   The PR body says what was wrong, what changed, and how it was verified
   (tests run and their result; anything not verified, marked as such).

Report back the issue and PR links.
