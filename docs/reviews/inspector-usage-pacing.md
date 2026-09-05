# Inspector Usage tab — account allowance pacing — handoff

Branch: `wks/workspacer-inspector-usage-pacing`. Base: `4c0b28bdf62d` ("fix(desktop):
above-pace usage reads as unfavourable, not as praise", local `master`). Delivery is
local commits on an isolated worktree branch — no push, no PR, no merge, no release,
no restart of the user's running app, and no change to any real configuration file
on this machine.

This is the follow-up the schedule branch listed as "authorized next feature,
deliberately NOT in this branch": the same paced provider windows, on the session
surface.

## What it does

The Inspector's Usage tab gains an **Account allowance** block under the session's
own figures: the provider plan limits this session spends from, with the same
5h/7d/monthly rows, the same expected-usage tick, the same `above pace` word in
`--wks-error`, and the same Settings → Usage schedule behind the weekly curve.

Nothing about the pacing is new code. It is the Overview's card, hook and mapper:

| Reused, unchanged | Where |
|---|---|
| once-a-minute shared report cache + 1s clock | `hooks/useUsageReport.ts` |
| pace arithmetic, ±2pp band, row descriptions | `lib/usagePacing.usagePacingRows` |
| verdict → (word, colour) | `lib/usagePacing.usagePaceLook` |
| the card itself, and its detail dialog | `components/UsageReportCard.tsx` |

`UsageReportCard` gained two additive props: `compact` (full width, 10px padding,
identical content) and `nowMs` (so a caller that has already asked "does this
account have a current window?" judges the rows against the same instant). The
inline stale/unavailable ternary moved to `usagePaceFallbackLabel` so both surfaces
name that absence identically.

## The one new decision: whose allowance is this?

`lib/usagePacing.usageReportAttribution(report, session)` →
`match | ambiguous | none | remote | unavailable`. The Overview never had to
answer this (it draws every account); a session surface does, and the wrong answer
is undetectable — another login's 91% under this session's name.

- **match** — exactly one row answers to the session's identity. For Claude that
  is the config root behind `transcriptPath` (`claudeAccountOf`) against
  `reportAccountKey`, the same vocabulary on both sides. For a provider with no
  per-session account marker it is the report having exactly ONE row for that
  provider, which is an identification rather than a guess.
- **ambiguous** — several rows and nothing picks one. Two Claude logins with a
  session that names neither; also two config roots whose basenames collide
  (`/a/work`, `/b/work` → one key), which a transcript path does not separate.
- **remote** — `snapshot.hub` is set. This report is the LOCAL hub's. Federation
  also blanks `transcriptPath`, which collapses to the DEFAULT account key, so
  without this check a peer's session would quietly borrow the local default
  login's numbers.
- **none** — the report was read and has nothing for this provider/account. The
  report's `null`-account (unattributed) bucket is never a match for a session
  that can name itself.
- **unavailable** — no report at all (an older hub, a failed fetch, a backend with
  no `usageReport`). The block renders NOTHING; the tab is byte-for-byte what it
  was.

Every non-match renders one muted sentence naming the reason, never a blank and
never a plausible number.

## Read first

- `apps/desktop/src/renderer/src/lib/usagePacing.ts` — `usageReportAttribution`,
  `usagePaceFallbackLabel`
- `apps/desktop/src/renderer/src/components/SessionAccountUsage.tsx` (new)
- `apps/desktop/src/renderer/src/components/claude/InspectorCard.tsx` — the Usage
  tab now ends with `<SessionAccountUsage session={session} />`
- Tests: `renderer/tests/components/inspectorAccountUsage.test.tsx` (new),
  `renderer/tests/usagePacing.test.ts` (attribution cases),
  `tests/e2e/usagePacing.test.ts` (`?surface=inspector`)

## Acceptance criteria

**Attribution**
- A session under `…/accounts/work/projects/…` draws the `work` row; the same
  Inspector re-pointed at a default-login session draws the default row and
  nothing of the first. No memory between the two.
- Two logins and no transcript path → the ambiguous sentence, no percentage, no
  bar. Two colliding roots → the same, even with a transcript path.
- A `hub`-tagged session → the peer-hub sentence, never the local default row.
- A named session is never folded into the unattributed bucket.
- A provider the report does not carry → the "no allowance reported" sentence.
- No report / a hub answering `null` → no block at all, and the tab's existing
  model, context bar, window bars, tiles and budget row are untouched.

**Currency and staleness**
- A window whose reset has passed is dropped (`usagePacingRows`' existing rule),
  and the heading says no window is currently running rather than standing over
  an empty card.
- `fresh === false` or `transport_stale` renders `Stale · pace unavailable` and no
  expected tick, with the percentage still shown.

**Lifecycle**
- Two open Inspectors + the Overview share ONE fetch per minute (the hook's
  subscriber set). Leaving the Usage tab unmounts the block; the last subscriber
  leaving clears both of the hook's timers (`vi.getTimerCount() === 0`).
- `refreshUsageReport()` — what the Settings schedule save calls — moves the
  Inspector's expected tick, not just the Overview's.

**Presentation**
- The block is headed `Account allowance` and says the figures are shared with the
  account's other sessions, not this session's own tokens.
- `above pace` resolves to the `--wks-error` rgb in Chromium, in light and dark, at
  360px and 1200px, in a 320px rail-width column, with no element overflowing its
  box and no horizontal page scroll.

## Checks

All passing at the final commit, Node 22 (the shell default, matching `ci.yml`):

- `npm run typecheck` (main + renderer)
- `npm run format:check` (Prettier, whole desktop tree)
- Desktop renderer suite: **181 files / 1719 tests passed** (was 180/1700).
- `npx playwright test --project=renderer usagePacing.test.ts` — 3 passed,
  including the new Inspector case; screenshots under
  `apps/desktop/test-results/usagePacing-inspector-usag-*/`, reproducible through
  `usage-pacing-harness.html?surface=inspector&theme=…`, which needs no app
  restart and no credentials.
- `npm run gen:changelog` regenerated `changelog.generated.ts`; its freshness test
  passes.

Not run: the desktop MAIN suite and the hub's Go suites — neither `src/main` nor
`services/` is touched by this branch.

**Runtime limitations, honestly.** The browser evidence renders the REAL
`InspectorCard` against a fabricated `window.electronAPI.usageReport`, not a live
hub or live provider credentials. No installed Electron app, no running claudemon,
no federated peer and no second real Claude login were exercised; the two-login and
peer-hub cases are proven against fixtures only. The TUI, the `/m` PWA and the web
client are untouched and still show the account windows only on Overview.

## Changed files

- `CHANGELOG.md`, `apps/desktop/src/renderer/src/lib/changelog.generated.ts`
- `apps/desktop/src/renderer/src/components/SessionAccountUsage.tsx` (new)
- `apps/desktop/src/renderer/src/components/UsageReportCard.tsx`
- `apps/desktop/src/renderer/src/components/claude/InspectorCard.tsx`
- `apps/desktop/src/renderer/src/lib/usagePacing.ts`
- `apps/desktop/src/renderer/src/harness/usagePacingHarness.tsx`
- `apps/desktop/src/renderer/tests/components/inspectorAccountUsage.test.tsx` (new)
- `apps/desktop/src/renderer/tests/usagePacing.test.ts`
- `apps/desktop/tests/e2e/usagePacing.test.ts`
- `docs/reviews/inspector-usage-pacing.md` (this file), `overview-usage-schedule.md`

## Follow-ups (not in this slice)

- The status-line window bars and the allowance rows can show the same 5-hour
  percentage twice on one tab, from two sources (this session's status line and
  the daemon's report) and with two colour semantics (severity vs pace verdict).
  Both are labelled and both were kept deliberately — the brief said preserve the
  existing metrics — but merging them is the obvious next simplification.
- A session whose login is ambiguous could be attributed if the snapshot carried
  the spawn's account/profile id directly, rather than only the transcript path.
  That is a wire/snapshot change and was explicitly out of scope here.
- No other client (TUI, `/m`, web `/app`) shows per-session allowance; they would
  each need this selector, not just the report.
