# Inspector Usage tab — two-section redesign — handoff

Branch: `wks/workspacer-clarify-usage-allowance-and-s`.
Base: `1eb2d24d88a2` ("docs(desktop): release note and reviewer handoff for
Inspector allowance", local `master`).
Commit: `df81395c` ("feat(desktop): two-section Usage tab, allowance first,
session second").

Delivery is a local commit on an isolated worktree branch. No push, no PR, no
merge, no release, no stash, no restart of the user's running app, and no change
to any real configuration file on this machine. The six unpushed commits that
were on local `master` at the start are untouched; this branch sits on top of
them.

Design source: `.workspacer/reports/2026-09-04-usage-display-design.md`, option
A ("two labeled sections in one Usage tab"), smallest complete slice.

## What changed, in one paragraph

The Inspector's Usage tab now answers two questions in a fixed order — **Account
allowance**, then **This session** — and draws the account's quota windows
exactly once. It previously drew the 5-hour window twice: from the hub's account
report (paced, `--wks-error` above the curve) and, a few rows lower, from the
opening session's live status line (a raw 50/80% severity ramp), with neither
labelled as account-wide. Account rows also gained the figure a reader actually
acts on, the remaining allowance, plus a spoken reset, an explained pace tick,
and an explicit "this is history" state.

## Acceptance criteria

Each row lists where the behaviour is pinned. `renderer` = `src/renderer/tests`,
`e2e` = `tests/e2e/usagePacing.test.ts` (Playwright `renderer` project).

| Criterion | Status | Pinned by |
|---|---|---|
| Account allowance section precedes a labelled `This session` section | met | `renderer/components/inspectorAccountUsage.test.tsx` "answers the account first…"; `e2e` |
| No Inspector shows two unlabelled 5-hour percentages or two bars with conflicting colour semantics | met | same test asserts one `usage-consumed` and no `5-hour limit`/`7-day limit` in the body; `e2e` same |
| Rows show valid used percentage, derived allowance left, reset and pace text | met | `renderer/usagePacing.test.ts` "derives the remainder only from a valid measurement", "speaks a near reset…"; `renderer/components/inspectorAccountUsage.test.tsx` "states the allowance left…" |
| Expected ticks disappear when stale or pace unavailable | met | `renderer/usagePacing.test.ts` "marks a reading historical…"; `inspectorAccountUsage.test.tsx` "calls a stale reading historical…" |
| Missing / expired / stale / ambiguous / remote / unavailable produce truthful text, no phantom 0% or 100% meter | met | the four pre-existing absence tests, plus new `100% left`/`0% used` count-0 assertions in `e2e` and the `left: undefined` cases in the unit test |
| Stale values explicitly historical, no pace marker | met | `usageStaleNote` unit test + `e2e` asserts the sentence renders |
| Claude monthly and provider-specific window shapes stay data-driven; absent 5h windows omitted | met | unchanged: `usagePacingRows` still iterates the reported `windows` map, no slot is hard-coded (`usagePacing.test.ts` "does not resurrect missing, removed, or unavailable windows") |
| Fits 320px and 360px without horizontal overflow, usable wide, light and dark | met | `e2e` runs 320/360/1200 × light/dark with a document-level `scrollWidth <= innerWidth` check and a per-descendant clipping sweep |
| Every meter and interactive card keyboard accessible, meaning without colour alone | met | account track is `role="meter"` with `aria-valuenow`/`aria-valuetext`; the tick has a worded legend; the session Detail affordance is a real `<button>` (keyboard activation is now native rather than a hand-rolled `onKeyDown` on a div); `e2e` opens it with Enter and closes with Escape |
| Overview behaviour, detail-dialog activation, recorded-cost fallback, context explanation, session budget intact | met | `usageSurfaces.test.tsx` and `usagePacingSurfaces.test.tsx` pass unmodified; full renderer suite green |

## Changed files

| File | Change |
|---|---|
| `apps/desktop/src/renderer/src/lib/usagePacing.ts` | `UsagePacingRow` gains `usedPct`, `left`, `resetPhrase`, `stale`. New `usageResetPhrase`, `usageStaleNote`, `USAGE_EXPECTED_LEGEND`, `USAGE_RESET_ABSOLUTE_AFTER_MS`. Pace arithmetic and the ±2pp band are untouched. |
| `apps/desktop/src/renderer/src/lib/sessionStats.ts` | `fmtResetIn(epochSecs, nowMs = Date.now())` — one added optional parameter, every existing caller unchanged. |
| `apps/desktop/src/renderer/src/components/UsageReportCard.tsx` | Row copy is `label · N% used · N% left · resets … · verdict`. Track becomes a `meter` with a full accessible name. Worded tick legend. Historical-reading note. |
| `apps/desktop/src/renderer/src/components/SessionAccountUsage.tsx` | Uses the shared heading; spacing flips to `marginBottom` now that it is first; one em dash removed from existing copy. Attribution and the four absence states are unchanged. |
| `apps/desktop/src/renderer/src/components/UsageSectionHeading.tsx` | **New.** One heading grammar for both sections, with an optional trailing action. |
| `apps/desktop/src/renderer/src/components/claude/InspectorCard.tsx` | `SessionAccountUsage` moves to the top of the tab; `This session` heading added with a `Detail` button; the `usageWindows(sl)` bar group is deleted; `fmtReset`, `usageWindows`, `fmtWindowLength` imports/helper removed. |
| `apps/desktop/src/renderer/src/components/claude/UsageDetailDialog.tsx` | Session-scoped `Account limits` gains a caption naming it live provider telemetry. Account scope unchanged. |
| `apps/desktop/src/renderer/src/harness/usagePacingHarness.tsx` | Two new Inspector columns: a stale provider observation and a running window with no measurement. Second window pushed past a day so the wall-clock reset phrasing is visible. |
| `apps/desktop/src/renderer/tests/usagePacing.test.ts` | +3 tests (remainder derivation, reset phrasing, historical marking). |
| `apps/desktop/src/renderer/tests/components/inspectorAccountUsage.test.tsx` | +4 tests (section order and single meter, allowance-left copy, stale, detail dialog). Existing `'70%'` assertions become `'70% used'`. |
| `apps/desktop/tests/e2e/usagePacing.test.ts` | 320px added to both card specs; new assertions for allowance-left, reset phrasing, legend, stale, unknown, absent duplicate bars, and the dialog. |

## Architectural constraints honoured

- **No backend, IPC or wire change.** `main/shared/usageReport.ts`, the hub, the
  daemon and the report contract are untouched. No spawn-time account-id work,
  which the design report scopes to a separate slice.
- **No web `/app`, `/m` or TUI change.** Those clients keep their current usage
  surfaces; adopting this hierarchy is listed as a follow-up.
- **No settings change.** The `Usage schedule` control and its plumbing are as
  they were; `inspectorAccountUsage.test.tsx`'s "follows a usage-schedule save
  through the shared cache" still passes, so the setting still reaches this
  surface.
- **One card grammar.** `UsageReportCard` remains the single account renderer
  for both the Overview and the Inspector, in the same `compact` variant. The
  new heading is a shared component precisely so the two sections cannot drift.
- **Theme tokens only.** Every colour added is `var(--wks-*)`
  (`--wks-text-secondary`, `--wks-accent`) or a `claudeColors` token. No literal
  theme colours; `format:check` and the repo's existing token conventions pass.
- **No em dashes in added prose**, and one pre-existing em dash removed from the
  copy this change was already editing.
- **Severity colouring stays where it means something.** The context-window
  meter keeps its `UsageBar` severity ramp — a filling context window is a real
  severity — while the account windows keep the pace mapper. The conflict was
  the two rendering the *same* fact differently, not the ramp itself.

### One defect found and fixed en route

`usagePacingRows` takes `nowMs` specifically so a window's currency question and
its answer come from one instant. `UsageReportCard` then formatted the reset
with `fmtResetIn(row.reset)`, which read `Date.now()` again — a row that survived
the currency check could count down from a different moment. The countdown is
now built inside the row against the caller's clock; `fmtResetIn` gained an
optional `nowMs` that defaults to `Date.now()`, so no other caller changes. This
surfaced as a failing new unit test under fake timers, not as a reported bug.

## Checks run

All from `apps/desktop`, all green:

| Check | Result |
|---|---|
| `npm run typecheck` (main + renderer) | pass |
| `npm run format:check` (prettier) | pass |
| `npm run test:renderer` (full suite) | 181 files, 1726 tests, pass |
| `npm run test:main` (full suite) | 148 files, 3156 tests, pass |
| `npx playwright test --project=renderer usagePacing` | 3 specs, pass |

The Playwright `renderer` project boots its own Vite server against
`usage-pacing-harness.html`; it needs no Go toolchain, no Electron and no xvfb.

## Runtime evidence, and its limits

**Verified by actually running a browser.** The Playwright spec drives real
Chromium over the harness at **320, 360 and 1200px, in both light and dark**, and
the assertions it makes there are ones jsdom cannot make: resolved custom
properties (the `above pace` word and its bar both compute to the `--wks-error`
token's rgb), `document.documentElement.scrollWidth <= innerWidth`, and a sweep
of every descendant for `scrollWidth > clientWidth` without `overflow-x: auto`.
Keyboard activation of the Detail affordance (Enter) and Escape-to-close were
exercised in that browser. I also opened the resulting full-page screenshots and
read them: `inspector-usage-{light,dark}-{320,360,1200}.png` under
`test-results/usagePacing-inspector-*/`. They show the two sections in order, one
account meter, the `70% used / 30% left / resets in 3h / above pace` row, the
worded tick legend, the stale column with its historical note and no tick, and
the missing-measurement column reading `Usage unknown` over an empty track. The
default focus ring on the Detail button is visible in both themes in those shots.

**NOT verified by running.** The real Electron app was not launched, and no
service was restarted — the task forbade it. So everything below is
code-and-test evidence only:

- Behaviour against a **live hub `usage.report`** payload. Every state here comes
  from fixtures shaped like the wire type, not from a real daemon reading real
  credentials.
- The **Overview pane** and the **agent-pane status bar** were not opened in a
  browser. They are covered by their existing jsdom suites, which pass unchanged,
  and the Overview's own e2e card spec passes at the newly added 320px.
- **Codex / Copilot / monthly-window** shapes. The code path is data-driven and
  the unit tests cover a report with only a `seven_day` window, but no real
  non-Claude account was observed.
- Screen-reader output. The `meter` role, `aria-valuetext` and the button's
  accessible name are asserted in the DOM; no assistive technology was run
  against them. Note the account card is itself `role="button"`, so some
  screen readers will flatten the meter into the card's name — the worded row
  text is what carries the meaning in that case, which is why it is worded.

## Follow-ups (not done here, deliberately)

1. Whether `/app`, `/m` and the TUI should adopt the same allowance/session
   hierarchy. They do not all have the Inspector's account selector.
2. A stable spawn-time account/profile id on the session snapshot, to retire
   transcript-path attribution ambiguity. Contract work, out of this scope.
3. `witness.select` returned empty output for every invocation in this session,
   including the no-argument git-diff form, so test selection here was done by
   reading the suites rather than by the tool. Worth a look if the tool is meant
   to be load-bearing for reviewers.
