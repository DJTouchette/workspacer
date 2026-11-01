# Project scratch

Per-project working files that are deliberately **not** in git but are still
worth keeping: personal test harnesses for projects that "don't test that way",
SQL that has to run on prod at some point, repro snippets, one-off probes.

## Why it is not a directory inside the checkout

Agent sessions mostly run in linked worktrees. Closing a session removes its
worktree with `git worktree remove` (no `--force`, `worktrees.rs` /
`worktreeService.ts`): git refuses when untracked-but-not-ignored files exist,
and **deletes ignored files** with the tree. So an ignored in-repo `scratch/`
either blocks cleanup or is silently lost; `git clean -fdx` loses it too. The
artifact cleanup deliberately never touches untracked source, so nothing
reclaims it either.

Scratch therefore lives outside every checkout, keyed by project:

```
~/.workspacer/scratch/<key>/
  project.json   {"version":1,"root":"/abs/project/root"}
  index.json     {"version":1,"items":{"<relative path>":{...metadata}}}
  files/         <- WORKSPACER_SCRATCH points here; the actual scratch files
```

## Project identity

`claudemon::scratch::project_root(cwd)` walks up from the canonical cwd to the
first `.git`:

- `.git` directory → that ancestor is the project root;
- `.git` file (linked worktree) → follow `gitdir:` then `commondir`; the main
  checkout (parent of the common `.git`) is the root, so every worktree of a
  repo shares one scratch;
- no `.git` → the canonical cwd itself.

The key is `<sanitized basename>-<16 hex FNV-1a of the root path>`: readable in
a file manager, collision-safe across same-named repos. It is pure filesystem
(no `git` subprocess) because it runs on every spawn.

## How agents find it

- **Env:** claudemon's `/sessions/spawn` and `/sessions/spawn-managed` set
  `WORKSPACER_SCRATCH=<…>/files` (never over a caller-supplied value) and create
  the directory. That one layer covers the desktop and hub launchers and every
  provider that receives spawn env (Claude PTY and stream, Codex, Copilot).
  - Only a cwd **inside a git repository** gets it, and never a project under
    the system temp dir: agents and test suites launched in throwaway
    directories would otherwise leave one empty scratch per temp dir behind.
    `scratch.save` still works for any directory.
  - **Gap:** OpenCode and Pi never receive spawn env today, so they get no
    variable; `list_scratch` returns `project.files` for them.
- **Instructions:** the launch instructions (hub `launch_instructions.rs`,
  desktop twin `mcpConfig.ts`) carry one static paragraph pointing at
  `$WORKSPACER_SCRATCH` and the `scratch.*` tools.

## Bus methods / MCP tools

Hub-owned (`services/hub-rs/src/services/scratch.rs`), operator tier like
`library.*` (not granted to view/triage tokens), ambient MCP tools for agents
(`list_scratch`, `read_scratch`, `save_scratch`, `mark_scratch`). Installed on
every hub with a home directory, including the Electron `--hub-only` control
plane (where `library.*` is served by desktop instead).

Parameters are strict: unknown keys and case-variant keys are refused. `path`
is relative with plain-name components only (max depth 8), and the resolved
target must be exactly `files/<path>` — no symlink below `files/` is followed
for a read, a write or a listing.

| Method | Purpose |
|---|---|
| `scratch.list {cwd}` | Walk `files/` (2,000 files / depth 8, symlinks skipped) and merge `index.json` metadata. Files agents wrote directly appear with `indexed:false`. Creates nothing; a damaged index is reported as `indexError` and never hides files. |
| `scratch.read {cwd, path}` | Text content, 5 MiB cap. Sensitive content is withheld from federated callers. |
| `scratch.save {cwd, path, content, purpose, …}` | Write a file + its metadata. `purpose` is required. Refuses to overwrite unless `overwrite:true`. |
| `scratch.mark {cwd, path, …}` | Update metadata only: `purpose`, `keep`, `sensitive`, `status`, `note`. |
| `scratch.pending {}` | Every project scratch with a pending file: `{projects:[{key, root, pending}], truncated}`, sorted by key. Reads each `index.json` only (1,000 project dirs, symlinks and damaged indexes skipped). |

Metadata: `purpose`, `sessionId`, `keep`, `sensitive`, `status`
(`draft` · `pending` · `done` · `archived`), `statusAt`, `note`, `createdAt`,
`updatedAt`. `status:"pending"` is how "this SQL still has to run on prod" is
recorded; `done` + `note` records when and how it ran.

Mutations publish `scratch.changed` `{key, pending}` — `pending` is that project's pending count after the write (not forwarded to federated peers).

## Sensitive content

- `sensitive:true` files are written `0600`.
- `scratch.save` scans content for credential shapes (URL userinfo,
  `password=`, private-key blocks, AWS access keys, bearer tokens). A hit forces
  `sensitive:true` and is reported in `warnings` — it fails safe, never refuses.
- Federated callers never receive sensitive content and may not clear the flag.
- **There is no execute method, by design.** The hub records that a script ran;
  running SQL against a database stays a human step.

## Native client

The native app has a **Scratch** page (sidebar inbox icon, `g n`):

- Opens on where you are looking — the filtered project, else the selected
  agent's folder, else the last project chosen, else the first project the hub
  knows — and stays on it while open; chips switch between known projects.
- Open / Pending / All filters; pending files sort first, with a warning border
  and a "N files are waiting for you to run them" line.
- A row opens to its actions (Mark done is primary on a pending file, Archive,
  Back to draft, Keep, Copy path, and Open / Show in folder when the hub is on
  this computer), an editable purpose and note, and the file's text read-only
  and highlighted by extension (first 256 KB). Sensitive content the hub
  withholds says so instead.
- `scratch.changed` is subscribed only once the page has opened, and refreshes
  the list and open file for the project shown.

Code: `apps/native/src/scratch.rs` (model), `apps/native/src/ui/scratch.rs`
(page); tests in `src/ui/tests/scratch_store.rs` and `tests/protocol.rs`.

## Not yet

- No automatic expiry: nothing in scratch is deleted by workspacer. (`keep`
  exists so a later "ephemeral" sweep has a fence to respect.)
- No delete method; delete files from disk.
- No desktop (Electron), TUI or mobile page yet; no sidebar badge for pending
  files before the page has been opened.
