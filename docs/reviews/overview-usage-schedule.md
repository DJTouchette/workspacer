# Overview usage schedule (five-day / seven-day) — reviewer handoff

Branch: `wks/workspacer-five-or-seven-day-pacing-sett-scji`. Base:
`ff21fc61d402` ("Add Overview account usage pacing from hub reports", local
`master`, unpushed). Delivery is local commits on an isolated branch for
independent review — no push, no PR, no merge, no release, no restart of the
user's running app, and no change to any real configuration file on this
machine.

## Two changes, one branch

1. The **usage schedule** setting (the dispatched scope).
2. A bounded **presentation correction** the user asked for mid-flight after
   seeing a Codex 7-day window at 89% with two days left rendered as *ahead* in
   amber: above-target consumption now reads as unfavourable.

## 1. The usage schedule

Settings → Session → **Usage schedule**, two values: *Work week (Mon–Fri)* and
*Every day (7 days)*, governing the SEVEN-DAY usage cards on Overview.

The correction that shaped the design: this is five ACTUAL weekdays, not the
half-weight weekend the earlier scout proposed. Under the work week, expected
consumption advances Monday to Friday and is FLAT across Saturday and Sunday.
Observed usage still counts whenever it happens.

## 2. The pacing presentation correction

`apps/desktop/src/renderer/src/lib/usagePacing.ts` gains `usagePaceLook`, the
single mapper from verdict to (label, colour). `UsageReportCard` uses it for
both the word and the consumed bar, replacing two independent ternaries.

- `ahead` → label **`above pace`**, colour `var(--wks-error)`.
  The old word read as praise, and the old colour (`--wks-warning`) is this
  app's NEEDS-YOU token — approval, input, stale — not its failure token.
  DESIGN_LANGUAGE.md assigns failure/danger to `--wks-error`.
- `on pace`, `under`, and no-verdict stay `var(--wks-text-secondary)`.
  Deliberately neutral rather than green: colouring every reading is how a
  meter trains people to ignore the one reading that means something.
- The accessible description becomes
  `Above pace · spending faster than expected`.

**No threshold or band moved.** The verdict still comes from the same inclusive
±2 percentage-point comparison of unrounded used-minus-expected, and the hub's
routing bands are not read by this file at all. This is presentation only.

`docs/reviews/overview-usage-pacing.md`'s acceptance line about the warning
colour is marked superseded in place rather than left silently false.

## Read first

- `services/hub/internal/limits/pace.go` — `CurveFiveDay`, `UsableCurve`,
  `weekendWeight`, `expectedShare`, `applyWeekendReserve`, `weightedSeconds`
- `services/hub/internal/usageprefs/usageprefs.go` — the store and the
  precedence table (`ApplySchedule`)
- `services/hub/cmd/hub/usageprefs.go`, `usagereport.go`, and the registration
  block in `cmd/hub/main.go`
- `apps/desktop/src/renderer/src/components/settings/UsageScheduleRow.tsx`
- `apps/desktop/src/renderer/src/hooks/useUsageReport.ts` (`refreshUsageReport`)
- `apps/desktop/src/renderer/src/lib/usagePacing.ts` (`usagePaceLook`) and
  `src/renderer/src/components/UsageReportCard.tsx`
- Tests: `internal/limits/paceweek_test.go`,
  `internal/usageprefs/usageprefs_test.go`, `cmd/hub/usageprefs_test.go`,
  `renderer/tests/components/usageScheduleRow.test.tsx`,
  `renderer/tests/components/usagePacingSurfaces.test.tsx` (new case),
  `renderer/tests/backend/backendParity.test.ts`, `main/ipc.test.ts`,
  `tests/e2e/usagePacing.test.ts` (new case)

## Precedence, stated once

| stored preference | what `usage.report` paces the 7-day window against |
|---|---|
| absent / unreadable / unrecognised | routing.yaml verbatim — including a hand-set `curve: workdays`. Byte-for-byte the pre-change behaviour. |
| `seven_day` | the CALENDAR curve, explicitly, overriding a hand-set `workdays`. Every other field (bands, bootstrap, timezone, enabled) stays the matrix's. |
| `five_day` | the FIVE_DAY curve: weekend weight zero, `spend_tail`, reserve 0. Bands, bootstrap, timezone and the enabled flag stay the matrix's. |

`five_day` neutralizes `weekend_weight` / `weekend:` / `weekend_reserve_pct`
because they answer a question a zero-weight weekend has already settled: with
no weekend budget there is nothing for a reserve to hold back, and scaling the
curve by a number set for a different curve would move the schedule the user
chose. `applyWeekendReserve` also refuses a reserve under `five_day` at the
arithmetic and says "IGNORED" in the explanation, so a hand-built config cannot
route around the store.

## Acceptance criteria

**Arithmetic**
- Under `five_day`, expected progress is identical at Friday 24:00, Saturday,
  Sunday and the last second before the reset, and reaches 100% at the end of
  the working week. It still climbs during Friday.
- Weekend consumption still moves the ratio; only the expectation pauses.
- Mid-week the curves order `five_day` > `workdays` > `calendar` for the same
  reading. The explanation names the curve that actually ran.
- A window whose reset falls mid-week (Wednesday→Wednesday) is weighted by real
  weekdays; nothing assumes a Monday-aligned week. The curve resumes on the
  Monday inside that window.
- Zero denominators: any 7×24h window weighs exactly 120 weekday hours wherever
  it starts, so `total` is never zero. A window whose ELAPSED part is entirely
  weekend yields `PaceUnknown` with a reason naming the curve — never an
  infinite or NaN ratio.
- Timezone is the hub's configured one; a nil location falls back to the
  calendar curve with the reason stated. The Santiago midnight DST transition
  terminates the day walk with a sane weekday total.
- `workdays` with `weekend_weight <= 0` still falls back to the calendar curve
  and still says so — the existing guard is unchanged, and `five_day` does not
  weaken it.
- The five-hour window's `PaceReport` is byte-for-byte equal under both
  schedules, and the monthly overage window is UNKNOWN under both.

**Preference store**
- Missing file → unset, no error, config unchanged. A nil `*Store` behaves the
  same, so a hub started without one needs no branch at any reader.
- Round trip persists at 0600, survives a reopen (restart), leaves no `.tmp`,
  and flips back the other way.
- Malformed JSON, an unknown word and an empty value all read as unset AND
  report a diagnostic error; the store stays writable so Settings can fix it.
- An invalid `Set` is refused and moves neither memory nor file.
- 8 concurrent writers × 8 concurrent readers settle on a valid value with `-race`.

**RPCs**
- `usage.setPacingSchedule` requires host authority; a view-tier caller is
  refused by name and nothing is written.
- `usage.pacingSchedule` is readable by the view tier (it is in
  `authtoken.viewMethods`) and accepts no parameters.
- A hub with no preference file reports `configurable: false` and refuses a save
  rather than accepting one it cannot persist.
- `usage.report` still refuses every caller parameter (`{"schedule":…}`,
  `{"curve":…}`, `{"url":…}`), is still in capspec's `inertMethods`, and its
  existing contract test is unmodified.
- Neither new handler names a write primitive; the store's only filesystem calls
  target the path it was constructed with.
- `routing.select` is untouched: it reads `Matrix.PaceConfig()` directly and
  never the preference.

**End to end**
- At a fixed clock on a Saturday, inside a Monday-opened window, against the
  REAL compiled-in matrix and the REAL store: unset and `seven_day` produce
  identical weekly pace; `five_day` produces the `five_day` curve and a strictly
  higher expected percentage; observed usage is unchanged; the five-hour window
  is identical across all three.

**Presentation**
- A window over the band renders the word `above pace` in `var(--wks-error)`
  and its consumed bar in the same colour; the word `ahead` appears nowhere.
- `on pace` and `under` render the word AND the bar in
  `var(--wks-text-secondary)`.
- Verified in Chromium as the resolved rgb of `--wks-error`, in light and dark,
  at 360px and 1200px — not only as the token string.
- The ±2pp band tests (`renderer/tests/usagePacing.test.ts`) are unmodified and
  still pass, which is the evidence the calculation did not move.

**Desktop**
- Backend parity: both methods are `HOST_ONLY` (bridged desktop → IPC → local
  hub) and implemented on the web backend against the selected hub; the remote
  backend uses its own hub and never local IPC.
- An older hub: the read answers `null` and the control renders dimmed with the
  reason; the WRITE answers `{ok:false,error}` and never a cheerful success.
  Neither ever falls back to the daemon or to `usage.report` with a parameter.
- A failed save keeps the previous selection, shows the hub's reason in an
  `alert`, and does NOT refresh the report.
- A successful save adopts the hub's own answer (not the value sent) and forces
  an immediate report re-read; a poll that started before the save cannot
  overwrite the post-save projection.
- CAP_LABELS and capspec both classify the two new methods; the hub-native
  registration floor moved 23 → 25 on both sides of the language boundary.

## Checks and runtime evidence

Run sequentially. All passing at the final commit:

- Hub: `gofmt -l .` clean, `go vet ./...`, `go test ./...` (all packages),
  `go test -race ./internal/usageprefs`.
- Desktop: `npm run typecheck` (main + renderer), `npm run format:check`
  (Prettier, whole tree), `npm run gen:changelog` freshness.
- Desktop main suite on Node 22: 148 files / 3156 tests passed.
- Desktop renderer suite on Node 22: 180 files / 1700 tests passed.
- Chromium: `npx playwright test --project=renderer usagePacing.test.ts` — 2
  passed, including the new schedule case, at 360px and 1200px in light and
  dark. Screenshots under
  `apps/desktop/test-results/usagePacing-usage-schedule-*/`; reproduce through
  `usage-pacing-harness.html?schedule=…`, which needs no app restart.

Node 22 throughout — the shell default here — matching `ci.yml`. The earlier
review recorded Node 26 producing unrelated renderer failures; not exercised.

**Runtime limitations, honestly.** The browser evidence uses the real
components against a fabricated `window.electronAPI`, not a live hub. The
hub-side end-to-end test runs the real projection, the real compiled-in matrix
and the real preference file at a fixed clock, but calls
`Snapshot.UsageReport` directly rather than through `time.Now()` in the bus
handler. No live provider credentials, no installed Electron app, no combined
deployed hub + claudemon stack, and no live weekend/DST wall-clock observation
were exercised. Rust, the TUI, the /m PWA and the standalone web client were
not touched or run; they inherit the hub behaviour through `usage.report` and
would need their own control to change the setting.

One visual near-miss worth recording: the first screenshot of the control
appeared to leave the highlight on the old option. It was the 120ms pill
cross-fade being captured immediately after the click, not a state bug — the
spec now settles the transition before asserting colours, and the jsdom test
asserts the highlight moves.

## Changed files

- `CHANGELOG.md`, `apps/desktop/src/renderer/src/lib/changelog.generated.ts`
- `apps/desktop/src/main/ipc.ts`, `src/main/ipc.test.ts`
- `apps/desktop/src/main/preload.ts`, `src/main/shared/ipcChannels.ts`,
  `src/main/shared/usageReport.ts`
- `apps/desktop/src/renderer/src/backend/bridgedBackend.ts`, `webBackend.ts`
- `apps/desktop/src/renderer/src/components/settings/UsageScheduleRow.tsx` (new),
  `SessionSection.tsx`, `primitives.tsx`
- `apps/desktop/src/renderer/src/harness/usagePacingHarness.tsx`
- `apps/desktop/src/renderer/src/hooks/useUsageReport.ts`
- `apps/desktop/src/renderer/src/lib/pluginPermissions.ts`
- `apps/desktop/src/renderer/src/types/electron.d.ts`
- `apps/desktop/src/renderer/tests/backend/backendParity.test.ts`,
  `tests/pluginPermissions.test.ts`,
  `tests/components/usagePacingSurfaces.test.tsx`,
  `tests/components/usageScheduleRow.test.tsx` (new)
- `apps/desktop/tests/e2e/usagePacing.test.ts`
- `docs/reviews/overview-usage-schedule.md` (this file)
- `services/hub/cmd/hub/main.go`, `usageprefs.go` (new),
  `usageprefs_test.go` (new), `usagereport.go`, `usagereport_test.go`
- `services/hub/internal/authtoken/authtoken.go`
- `services/hub/internal/capspec/capspec.go`, `composition.go`,
  `hubnative_test.go`
- `services/hub/internal/limits/pace.go`, `paceweek_test.go` (new)
- `services/hub/internal/routing/pacing.go`, `routing.default.yaml`
- `services/hub/internal/usageprefs/usageprefs.go` (new),
  `usageprefs_test.go` (new)

## Follow-ups (not in this slice)

- **Authorized next feature, deliberately NOT in this branch:** the same
  provider-window pacing in the Inspector's Usage tab. To be dispatched
  separately once this change is reviewed and landed.

- No client other than the desktop can CHANGE the schedule yet. The web `/app`
  and `/m` read `usage.report` and would show the new curve, but neither has a
  control; the setter is trusted-only, so a phone token could not use one
  without a deliberate tier decision.
- `routing.yaml` now accepts `curve: five_day` by hand (the validator and the
  arithmetic agree on the vocabulary), which means `routing.select` CAN be put
  on the five-day curve by an operator editing that file. That is the existing
  operator authority, not something this feature grants a bus caller.
- The store has no hand-edit reload (jobs.json-style polling). A file edited
  behind the hub's back applies on the next hub start.
