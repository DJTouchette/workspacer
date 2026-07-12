# Workspacer Product Tightening Tasks

This backlog turns the current product review into concrete cleanup work. The
goal is not to add more breadth. The goal is to make the core loop feel obvious,
safe, and finished:

> spawn agent -> monitor attention -> approve/answer -> review changes -> commit/push

## Product Principles

- Make Sidebar + Triage Inbox the default mental model.
- Treat Fleet Deck, Agents Monitor, supervisors, plugins, remote server mode, and
  headless operation as power-user surfaces.
- Close existing loops before adding new panes or agent features.
- Prefer safer defaults for first-run users, with explicit opt-in for powerful
  modes.
- Keep `docs/features.md` as the source of truth and remove stale maturity claims
  elsewhere.

## P0 - Lock The Core Story

### Define The First 10 Minutes

Tasks:

- Write a short first-run scenario: open app, spawn first agent, answer/approve,
  review changed files, commit/push.
- Audit the UI against that scenario and mark every non-essential surface as
  primary, secondary, or advanced.
- Make sure the welcome/onboarding copy teaches only the core loop plus how to
  find the command palette.

Acceptance criteria:

- A new user can understand the app without learning Fleet Deck, supervisors,
  plugins, remote server mode, or the TUI.
- The first-run path has one obvious CTA: spawn an agent.

## P1 - Close The Important Loops

### Remote Pairing Should Use Scoped Tokens

Status: implemented.

Problem:

- The token system supports `view`, `triage`, and `operator`, but the sharing UI
  still hands out the operator token by default.
- Phone pairing should not grant full host control unless the user explicitly
  chooses that.

Tasks:

- [x] Add a scope selector to the Remote Control dialog: `Triage phone`
  (recommended), `Read-only`, `Full control`.
- [x] Default QR/mobile pairings to `triage`.
- [x] Generate or select a scoped token for the chosen scope instead of reusing the
  implicit operator remote token.
- [x] Make the warning text scope-aware.
- [x] Add a visible token/device list with revoke actions, or link clearly to the CLI
  until the UI exists.
- [x] Decide whether revoke should actively close existing connections; if not,
  state that clearly in the UI.

Acceptance criteria:

- [x] A phone QR code does not grant spawn/git/plugin/admin access by default.
- [x] The full-control path is still available but clearly marked as powerful.

### Finish Review/Merge Action Wiring

Status: implemented.

Problem:

- Review can stage, commit, and push, but merge and `review_diff` next-actions
  are called out as not built.

Tasks:

- [x] Wire `review_diff` next-actions into the Review pane or Inbox.
- [x] Add a merge/conflict flow if the classifier or backend emits a merge action.
- [x] Surface git errors in actionable language, especially conflicts, no upstream,
  nothing staged, and push rejection.
- [x] After commit/push, clearly return the user to the agent or mark the attention
  item resolved.
- [x] Add focused tests for review action routing and git error states.

Acceptance criteria:

- [x] A finished agent can be taken from "done" to reviewed, committed, and pushed
  without dropping to a shell.
- [x] Merge/review next-actions do not point at dead UI.

### Safer First-Run Agent Defaults

Status: implemented.

Problem:

- New Claude sessions currently default to full/bypass permissions. That is
  convenient for the author but risky for new users.

Tasks:

- [x] Change first-run defaults to ask/approve before tool access, or add an explicit
  first-run choice.
- [x] Keep a clear opt-in for "full access" with a warning.
- [x] Persist the user's choice after they opt in.
- [x] Re-check managed providers so the permission labels map cleanly across Claude,
  Codex, OpenCode, and Pi.

Acceptance criteria:

- [x] A first-time user does not accidentally spawn a full-access agent.
- [x] Power users can still make full access their default.

## P2 - Reduce Surface Area Pressure

### Make Fleet Surfaces Hierarchical

Status: implemented.

Problem:

- Sidebar, Triage Inbox, Fleet Deck, Agents Monitor, Ask the Fleet, and
  supervisor agents all compete as "fleet" concepts.

Tasks:

- [x] Declare Sidebar + Inbox as primary.
- [x] Move Fleet Deck copy/commands toward "overview/power view" language.
- [x] De-emphasize Agents Monitor unless subagent/workflow monitoring is active.
- [x] Avoid presenting Fleet Deck and Agents Monitor as separate must-learn concepts
  during onboarding.

Acceptance criteria:

- [x] Users can ignore Fleet Deck and Agents Monitor without feeling like they missed
  the main product.

### Collapse Ask The Fleet Duplication

Status: implemented.

Problem:

- There are multiple ways to ask/spawn a supervisor: `Ask the Fleet`,
  `Spawn Fleet Agent`, and "Just spawn a fleet agent".

Tasks:

- [x] Keep one primary command: `Ask the Fleet`.
- [x] Move "spawn supervisor without a question" into an advanced/secondary affordance.
- [x] Consider hiding non-Claude supervisor providers until they are no longer marked
  experimental. Decision: keep them in the Ask pane only, still marked experimental.
- [x] Make supervisor agents read as an implementation of Ask, not a separate product
  mode the user must understand.

Acceptance criteria:

- [x] The command palette does not show two equally prominent supervisor entry
  points.

### Tame The Spawn Dialog

Status: implemented.

Problem:

- The spawn dialog is well organized, but it exposes many expert controls at
  once: model, transport, resume, profile, MCP, effort, worktree, permissions.

Tasks:

- [x] Keep working directory + provider as the primary path.
- [x] Hide or collapse advanced pills by default for first-run users.
- [x] Keep dangerous/powerful settings visible when active, especially full access.
- [x] Consider a "remember my advanced settings" behavior for power users.
  Decision: remember whether the Advanced section was last left open.

Acceptance criteria:

- [x] A new user can spawn an agent by choosing only a directory.
- [x] Expert controls remain reachable without bloating the first-run experience.

### Simplify Remote Product Language

Status: implemented.

Problem:

- The repo supports desktop, phone PWA, `/remote`, full web app, TUI, headless
  server, and desktop-as-client. That is technically strong but hard to explain.

Tasks:

- [x] Product copy should lead with "desktop plus phone".
- [x] Move `/remote`, `/app`, TUI, and headless server into advanced docs.
- [x] In the Remote Control dialog, distinguish "share this machine" from "connect
  this app to another server".

Acceptance criteria:

- [x] A normal user understands the remote story as phone access first.

## P3 - Fix Documentation Drift

### Refresh Component READMEs

Tasks:

- [x] Update `services/claudemon/README.md` so `init`, `watch`, PTY, transcript, and
  managed-provider claims match the current code.
- [x] Update `services/hub/README.md` so MCP/adopt/headless capabilities match
  `docs/features.md`.
- [x] Update `apps/desktop/README.md` so it reflects the current pane set and remote
  model.
- [x] Update `apps/tui/README.md` maturity/test language after adding tests.

Acceptance criteria:

- [x] No README calls a working feature a stub.
- [x] `docs/features.md` remains the most detailed maturity catalog, and other docs
  point to it instead of contradicting it.

### Add A Lightweight Docs Drift Check

Tasks:

- [x] Add a simple grep/script check for stale phrases like "still stubs",
  "planned", "next milestone", and "not implemented" in component READMEs.
- [x] Make the script informational at first, then decide whether to wire it into CI.
  Decision: keep it out of CI for now; run `make docs-drift` before release, or
  set `WKS_DOC_DRIFT_STRICT=1` if we want it to fail a future CI job.

Acceptance criteria:

- [x] Stale maturity language is easy to catch before release.

## P4 - Raise Test Confidence Where It Matters

### Multi-Client End-To-End Check

Tasks:

- [x] Add an E2E scenario with desktop/web or headless/web sharing the same layout.
- [x] Verify one agent appears once across clients.
- [x] Verify approval/question resolution from one client updates the other.
- [x] Verify terminal attach/replay after reconnect.

Acceptance criteria:

- [x] Cross-client de-duplication and attention resolution are covered by a real
  integration-style test.

### TUI Test Expansion

Tasks:

- Add tests beyond key encoding: navigation, approval/question handling, review
  pane basics, reconnect behavior.
- Cover direct mode and bus mode separately if feasible.

Acceptance criteria:

- TUI maturity is no longer blocked on "only key-encoding units".

### Main-Process Service Tests

Tasks:

- Prioritize tests around remote sharing, token scope handling, hub adoption,
  daemon restart, plugin install/remove, and review/git operations.
- Remove or refresh stale E2E tests that no longer reflect the current Claude
  pane.

Acceptance criteria:

- The highest-risk app services have behavioral tests, not only renderer tests.

## P5 - Park Or Remove Old Substrate

### Decide The Fate Of The Parked Inbox/Classifier Stack

Problem:

- The product has chosen per-agent workspaces, while classifier/items tables and
  APIs remain parked.

Tasks:

- Keep only the pieces that actively feed the current Triage Inbox or future
  review actions.
- Mark the rest as internal experimental code, or remove it.
- If kept, document the intended trigger for reviving it.

Acceptance criteria:

- There is no ambiguity between "per-agent workspace" and the older "inbox of
  decisions" product direction.

## Suggested Order

1. Remote scoped pairing.
2. Safer first-run permissions.
3. Collapse Ask/Fleet duplication.
4. Finish review/merge next-action wiring.
5. Refresh stale docs.
6. Add multi-client and TUI confidence tests.
7. Reassess parked substrate.
