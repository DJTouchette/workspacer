# Contributing to Workspacer

Thanks for wanting to help. Workspacer is a monorepo with four moving parts —
an Electron/React desktop app, a Rust session daemon, a Go control plane, and a
Rust terminal client — so this guide gets you from a clean checkout to a green
build and a mergeable pull request.

By contributing, you agree to the terms in [Contributor terms](#contributor-terms)
at the bottom of this file.

## Ground rules

- **Be respectful.** Assume good faith, keep discussion technical, and
  remember there's a person on the other end of the review.
- **Small, focused PRs** get reviewed faster than large ones. If a change is
  big or architectural, open an issue first so we can agree on the shape before
  you write it.
- **Don't file security issues as public issues.** See
  [`SECURITY.md`](SECURITY.md) for how those are handled.

## Prerequisites

Toolchains are pinned with [`mise`](https://mise.jdx.dev) via `mise.toml`:

- **Go 1.25** — the `hub` control plane
- **Node 22** — the desktop app (Electron + React + Vite)
- **Rust** (stable, via `rustup`) — `claudemon` and `wks-tui`

If you use `mise`, run `mise install` to get Go and Node at the pinned
versions. Install Rust separately with `rustup`.

## First build

From the repo root:

```bash
make install          # desktop JS deps (root + renderer workspaces)
make build            # build all four components (desktop, hub, claudemon, tui)
```

Then run the app in dev mode:

```bash
make build-claudemon  # required once before agents can spawn
make dev              # Vite renderer + Electron with hot reload
```

The desktop app spawns and supervises `claudemon` and `hub` for you — you don't
start them by hand. See the [README](README.md#common-tasks-from-the-repo-root)
for the full `make` target list, and the per-component READMEs
(`apps/desktop/`, `apps/tui/`, `services/claudemon/`, `services/hub/`) for
building each piece on its own.

## Where things live

| Path                 | What it is                                                        |
| -------------------- | ---------------------------------------------------------------- |
| `apps/desktop/`      | Electron + React desktop client (the primary GUI)                |
| `apps/tui/`          | `wks-tui`, the Rust terminal client                              |
| `services/claudemon/`| Rust session daemon: sessions, PTYs, provider adapters, git      |
| `services/hub/`      | Go control plane: event bus, supervisor, plugins, MCP facade     |
| `docs/`              | design notes, specs, and the feature catalog (`features.md`)     |
| `landing/`           | the static marketing site + user docs                            |

New to the codebase? Good reading order: [`docs/features.md`](docs/features.md)
(what exists and how mature it is), then `landing/docs.html` (the user guide),
then `landing/build.html` (the architecture and hub-bus protocol).

## Making a change

1. **Branch** off `master` (or fork, if you're external):
   `git checkout -b my-change`.
2. **Write the change with tests.** Match the style and structure of the code
   around it. Add or update tests for anything with runtime behavior.
3. **Format and lint** the components you touched (see below).
4. **Run the tests:** `make test` runs the desktop + hub + tui suites.
5. **Commit** in logical chunks with clear messages (see below).
6. **Open a PR** against `master` describing what changed and why, with a note
   on how you verified it.

### Formatting & linting

**The format gates are separate CI jobs and no test suite covers them.**
`make test` can be entirely green on a tree that CI rejects on formatting
alone — that is not hypothetical, it has reddened `master`. There is no
`make fmt` target, so run the four checks by hand for whatever you touched:

| Component | Check CI runs | Fix it with |
| --- | --- | --- |
| `apps/desktop` (TS/TSX) | `npm run format:check` (Prettier) | `npm run format` |
| `services/claudemon` (Rust) | `cargo fmt --check` | `cargo fmt` |
| `apps/tui` (Rust) | `cargo fmt --check` | `cargo fmt` |
| `services/hub` (Go) | `test -z "$(gofmt -l .)"` | `gofmt -w .` |

Each runs from that component's directory. Note that `cargo fmt --check` is
run **twice**, once per Rust crate — they are separate workspaces and
formatting one does not format the other.

Also gated in CI, and also not covered by the tests:

- `npm run typecheck` in `apps/desktop`.
- `cargo clippy --all-targets -- -D warnings` for both Rust crates — the tree
  is clippy-clean, so warnings are hard errors.
- `go vet ./...` in `services/hub`.
- **Generated files must be fresh.** `configDefaults.generated.ts` and
  `changelog.generated.ts` are regenerated in CI and the job fails if the
  tree moves. If you edited `services/hub/cmd/brain/config_defaults.json` or
  `CHANGELOG.md`, run `npm run gen:config-defaults` / `npm run gen:changelog`
  in `apps/desktop` and commit the result.
- **No unresolved conflict markers**, anywhere in the tree. This job exists
  because `landing/` sits inside no other job's blast radius and once shipped
  conflict markers to the live site.

There's a `.git-blame-ignore-revs` at the root — bulk-formatting commits are
listed there so `git blame` stays useful. If you do a repo-wide reformat, add
the commit hash to that file.

### Tests

- `make test` runs desktop + hub + tui.
- Prefer testing behavior end-to-end where a change has a runtime surface, not
  just the happy path.
- If you're changing a daemon or the hub bus, exercise the affected flow, not
  only the unit under it.

### Commit messages

- Keep the subject line short and imperative ("Add review-pane push button",
  not "Added" / "Adds").
- Explain the *why* in the body when it isn't obvious from the diff.
- Reference an issue number when there is one.

### Pull requests

- Target `master`.
- Describe the change, the reasoning, and how you verified it.
- Keep the diff scoped to one concern; split unrelated changes into separate PRs.
- Expect review comments — that's the normal path to merge, not a rejection.

## Cutting a release

Releases are cut by hand. **Nothing about the process is automatic on a push
to `master`** — `.github/workflows/release.yml` triggers on `v*` tags,
`pull_request`, `workflow_dispatch` and a nightly `schedule`, and on nothing
else. Pushing to `master` runs `ci.yml` only, which never builds an installer.

### Versioning

The release version is the desktop app's version in
`apps/desktop/package.json`. The tag is that version with a `v` prefix
(`0.150.0` → `v0.150.0`), and `scripts/changelog-section.mjs` looks the
version up in `CHANGELOG.md` by that number, so the three have to agree.

### CHANGELOG.md is the single source for release notes

Everything derives from it and nothing else is hand-maintained:

- the **GitHub release body**, cut at tag time by
  `scripts/changelog-section.mjs`;
- the app's **Settings → Updates** pane and its post-update "what's new"
  notice, via `apps/desktop/src/renderer/src/lib/changelog.generated.ts`;
- the **nightly** release body, which is the `[Unreleased]` section — because
  that is exactly what a nightly is.

Write entries by what a *user* notices, not by commit. Then regenerate:

```bash
cd apps/desktop && npm run gen:changelog
```

and commit `changelog.generated.ts` alongside the markdown. CI fails the
desktop job if you forget, and a drift test (`changelog.test.ts`) fails too.
`gen-changelog.mjs` also **refuses a release heading with no entries under
it** — an empty section renders as an empty card in the app — and
`changelog-section.mjs` **exits nonzero when the tagged version has no
section**, which fails the tag build rather than publishing an empty release
page.

### The steps

1. **Get `master` green**, including the format gates above — `make test`
   does not cover them.
2. **Write the `[Unreleased]` section** in `CHANGELOG.md`, then promote it by
   renaming the heading to `## [X.Y.Z] - YYYY-MM-DD`. Do **not** leave an
   empty `[Unreleased]` behind — the generator refuses a heading with nothing
   under it. Start the next one when the first entry exists, directly below
   the file's prose header.
3. **Bump `apps/desktop/package.json`** (and its lockfile) to `X.Y.Z`, run
   `npm run gen:changelog`, and commit both with the CHANGELOG edit.
4. **Tag and push the tag:** `git tag vX.Y.Z && git push origin vX.Y.Z`.
5. **Wait for all three build legs** (macOS, Windows, Linux). Each attaches
   its own installers, the `latest*.yml` + `.blockmap` updater metadata, and
   the standalone `workspacer-server-*` / `workspacer-claudemon-*` bundles to
   the same release.
6. **Publish the draft by hand.** This is the step that is easy to miss:
   `release.yml` creates the GitHub Release with `draft: true`. A draft is
   invisible to `api.github.com/repos/.../releases/latest`, which is the
   endpoint the landing page's download button reads — so until a human hits
   **Publish release**, the site still offers the previous version and the
   auto-updater sees nothing new.
7. **Check the release page** actually has every platform's installer and the
   updater metadata before you announce it. `fail_on_unmatched_files: false`
   means a leg that produced no artifact does not fail the run.

### Nightlies

The rolling `nightly` prerelease rebuilds `master` on an 08:00 UTC cron, and
the gate job skips the build when nothing has landed since the last one. To
force one, dispatch it manually:

```bash
gh workflow run release.yml -f nightly=true
```

or use **Actions → release → Run workflow** and tick `nightly`. A plain
`workflow_dispatch` **without** `nightly=true` is only a build smoke-test: it
uploads artifacts and publishes nothing.

Nightly is a prerelease on the fixed `nightly` tag, rolled publish-last so a
failed build leaves yesterday's nightly live. It *does* ship updater metadata,
so nightly installs self-update onto the next nightly — and never onto stable,
since a stable install resolves `/releases/latest`, which excludes
prereleases. Going back to stable means installing a release build by hand.

### Known gaps, so nobody rediscovers them

- **macOS builds are unsigned and un-notarized** (`CSC_IDENTITY_AUTO_DISCOVERY:
  false`), and there is no Intel leg — Apple Silicon only. Windows *is* signed
  on tag builds via Azure Trusted Signing (see
  [`docs/windows-code-signing.md`](docs/windows-code-signing.md)).
- **macOS auto-update does not work.** No mac zip is published, which is what
  `MacUpdater` needs, and it would not be trustworthy unsigned anyway.
- `scripts/check-doc-drift.sh` is informational unless `WKS_DOC_DRIFT_STRICT=1`
  and is **not wired into CI** — it runs via `make docs-drift`. It only greps
  four component READMEs for maturity words; it does not look at `landing/` or
  `docs/`.

## Reporting bugs & requesting features

- **Bugs:** open an issue with what you did, what you expected, what happened,
  and your OS + which agent backend (Claude Code / Codex / OpenCode) you were
  running. Logs or a minimal repro help a lot.
- **Features:** open an issue describing the use case before writing code, so we
  can talk through the design. `docs/features.md` shows what's already built.

## Contributor terms

Workspacer is distributed under the [MIT License](LICENSE).

By submitting a contribution (a pull request, patch, or any other work) you agree
that:

1. You are legally able to make the contribution, and it is your original work
   (or you have the right to submit it).
2. You license your contribution to the project under the MIT License, the same
   terms as the project as a whole.

If your employer has rights to work you create, make sure you have permission to
contribute before you do. If we ever adopt a formal Contributor License Agreement
(CLA), we'll ask you to sign it; until then, this section is the agreement.

This is not legal advice. If any of the above is a problem for your situation,
open an issue or email **djtouchette1993@gmail.com** before contributing.
