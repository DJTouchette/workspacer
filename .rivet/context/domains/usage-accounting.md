---
title: Usage Accounting: Twin Pricing Tables, Subagent Transcripts & Rate-Limit Windows
tags: [usage, pricing, cost, rate-limits, cross-language, twin-structure, subagents, codex, tokens, report]
related_paths:
  - "apps/desktop/src/main/services/modelUsage.ts"
  - "apps/desktop/src/main/services/analyticsBackfill.ts"
  - "apps/desktop/src/main/services/sessionStore/usageAccumulator.ts"
  - "apps/desktop/src/main/services/sessionStore/analyticsWriter.ts"
  - "services/claudemon/src/session/usage.rs"
  - "services/claudemon/src/session/pricing.rs"
  - "services/claudemon/src/session/state.rs"
  - "services/claudemon/src/session/usage_report.rs"
  - "services/claudemon/src/session/account_usage.rs"
  - "services/hub/cmd/hub/usagereport.go"
  - "apps/desktop/src/renderer/src/hooks/useUsageReport.ts"
  - "apps/desktop/src/renderer/src/lib/usagePacing.ts"
  - "apps/desktop/src/main/headless/analytics.ts"
owner: Damien Touchette
last_reviewed: 2026-09-26
---

# Usage Accounting

## Three measurements, separate evidence

Keep cumulative billed tokens/cost, current context occupancy, and account quota
windows separate. A long session can have millions of cumulative tokens without
holding that many tokens in its current context. A model’s catalog capacity is
not proof of the active window, and a window’s last recorded percentage is not
proof it describes the window open now.

There are two costing implementations: desktop/shared TypeScript
`modelUsage.ts` with its live accumulator and analytics folds, and claudemon
Rust `usage.rs`/`pricing.rs` with managed-provider `UsageAcc`. The TUI consumes
published usage; it does not maintain a third price table.

## Pricing and cache accounting

`MODEL_RATES` in TypeScript and `BUILTIN` in Rust are software estimates, not a
live provider billing feed. Both perform longest-prefix matching and read
`~/.workspacer/model-rates.json`; an override wins an equal-length prefix tie.
Rust transcript costing delegates lookup to `pricing.rs`. The TS override
writer updates the same file, and both readers cache by mtime.

The delimited `claude-opus-4-1-` prefix is present in both tables. Its delimiter
prevents matching 4.10–4.19; dated 4.0 IDs need their separate prefix. Maintain
`contracts/model-pricing-cases.json` and both loaders when changing rates or
matching rules rather than reintroducing a documented-but-fixed divergence.

Claude-transcript pricing falls back to this build’s price-only defaults for
an unknown model (3/15 USD per million input/output tokens). Managed generic
`estimate_cost` returns None for an unknown model. Neither fallback invents a
context-window size. Copilot’s token-based session dollar estimate is not the
same measurement as its recorded GitHub AI-credit charge.

Claude cache-write cost uses the reported 5-minute/1-hour token split, at 1.25×
and 2× input rate respectively. Unitemized writes use the 1-hour rate. Cache reads
use the explicit cached-input rate where supplied, otherwise the read multiplier.
The generic managed estimator treats cached input as a subset of total input,
clamps it to that total, and has no Claude cache-write bucket. Do not apply
Claude’s write formula to every provider’s input shape.

`StatusLine.cached_input_tokens` now carries the managed cache-read count.
Desktop status mapping, brain projection, and TUI types retain it; the old claim
that Codex’s cache count is discarded before the UI is obsolete. An absent
cache count means not reported, not zero. Claude transcript cache splits remain
separate from the managed status-line subset.

## Transcript folds and analytics

`usageAccumulator.ts` folds live assistant usage; `analyticsUsage.ts` and
`analyticsBackfill.ts` recompute historical usage; Rust `usage.rs` folds daemon
transcripts and sibling subagent files. Preserve message-ID deduplication and
fallback identifiers when replaying partial/repeated transcript entries. Missing
identifiers cannot provide the same deduplication guarantee as a stable ID.

Subagent/sidechain turns contribute cumulative tokens, cost, and per-model totals
at their own model’s rates. They must not replace the parent’s current context
occupancy or active model. Per-model history rows must be cleared/replaced
consistently during recomputation so obsolete slices do not double-count totals.
Backfill markers are versioned; rerunning an old fold is not proof its output
matches a new pricing algorithm.

Desktop history uses the shared session-history schema and model split table.
The shared Rust backend owns `analytics.summary` and `analytics.recent` when it
owns an execution engine, persisting separate `headless-analytics.sqlite`
history. Catalog-only composition leaves those capabilities to the desktop;
an unavailable local engine must not shadow that provider. The Rust observer
also records terminal session evidence without waiting for an analytics query.
Missing or failed source reads remain availability errors, never measured zeros.

Legacy analytics columns defaulted to zero without recording whether a sample
was measured. Consumers such as `useSessionAnalytics` treat those stored zeros
as unrecorded and show absence. That convention must not be copied onto the
new tagged usage report, where an explicitly measured zero is a valid answer.

## Context window and runtime health

Canonical selection is `{model, contextWindow}` (snake-case on the daemon wire),
with a legacy model-string projection for compatibility. Normalize suffix
spellings at ingress; preserve the canonical owner fields through persistence
and transport instead of reconstructing them from renderer settings.
`contracts/model-context-windows.json` pins normalization, provider argv and
window resolution across TypeScript, Rust, and Go.

Window resolution distinguishes reported runtime capacity, user override,
requested capacity, and the model table. Observed occupancy can disprove a
candidate beyond the drift tolerance; it cannot promote an unknown window to a
larger guessed capacity. If no claim survives, the window is unknown.

A reported percentage and denominator are one claim. Reject a disproved pair
together; retaining its percentage while substituting a different denominator
manufactures a new reading. Display rejection does not rewrite stored source
fields. `contextTokensFromStatusLine` is a derived display estimate
(clamped percentage × reported window), not a direct provider token count.

Automatic context-health actions use `ContextHealth`, a separate
runtime-confirmed sample with used/window tokens, percentage, runtime provenance,
observation time, provider, and epoch. Requested/catalog capacity is not a health
sample. The epoch is a decimal string on the wire because a u64 can exceed
JavaScript’s exact integer range. Provider/session/model boundaries fence samples;
an explicit invalidating observation differs from a tick with no context evidence.
See `contracts/context-health-cases.json` and the store’s tri-state update handling.

Codex spawn can request a window through `model_context_window`; its requested,
catalog, runtime, cumulative-token, and compaction-threshold values remain
different quantities. A picker request is provisional until runtime evidence
confirms it. Do not infer a live context allocation from a large lifetime token
total or from a catalog maximum alone.

## Session-free account reporting

Claudemon `GET /usage/report` builds account/provider rows without needing a
running session. `usage_report.rs` uses a tagged `Measured` scalar:

| State | Meaning |
| --- | --- |
| `ok` with value, including zero | A reading exists |
| `unknown` with reason | Not known now; a later read may succeed |
| `unavailable` with reason | This source cannot supply the measurement |

`account: ""` identifies the default Claude root; `account: null` is unattributed.
Do not merge null into the default account. Rows carry source, observation/freshness
information, windows, spend, token splits, and per-model readings. Provider data
sources differ:

- **Claude:** account OAuth usage plus transcript-derived estimates. Config-root
  identity separates configured logins, including idle profiles.
- **Codex:** disk rollout rate-limit evidence and available disk token records.
  A last-turn reading can exist at boot but already be expired; report spend is
  not a native metered-dollar figure.
- **Copilot:** local usage/charge records supply token/AI-credit information.
  This implementation reports unavailable quota headroom where no usable source
  exists. Recorded usage does not by itself identify the login that incurred it.

Provider disk schemas and API behavior are version-sensitive. A dated CLI probe
is useful historical evidence, not a permanent claim about every installed
provider version. Prefer the current readers in `providers/codex_usage.rs` and
`providers/copilot_usage.rs` over manually querying an old schema from a note.

## Claude polling and account attribution

`account_usage.rs` discovers configured roots as well as live-session roots when
boot polling is enabled. `WORKSPACER_USAGE_POLL_ON_BOOT` defaults on; explicit
false/0/off/no disables idle-root discovery, not live-root polling. Desktop and
`workspacer serve` pass the config preference to the daemon.

Healthy live roots poll every minute and idle roots every 15 minutes. Failures
back off to one hour. The scheduler checks due roots every 30 seconds and
rearms an idle-to-live root promptly. Account freshness lasts the idle cadence
plus one live cadence (16 minutes), so a healthy idle poller does not repeatedly
invalidate its own reading between polls. This age gate and quota-window reset
currency answer different questions.

Credential failure/expiry leaves the attempt without a reading; it is not
proof of zero usage. The poller does not refresh the provider’s login on behalf
of the CLI. Session `config_root` spawn facts are the stronger account evidence;
transcript-root attribution must preserve the path spelling, because profiles
can symlink their projects into a shared physical directory. Realpath can erase
which account wrote a transcript. Old history lacking attribution stays unknown
rather than being assigned to the currently selected login.

## Hub projections and renderer refresh

The hub’s no-parameter `usage.report` reads the existing usage sampler, routing
matrix and pacing preference. It returns server-computed projections with a
validity deadline; it does not select a model, spawn work, or accept caller-supplied
usage/capacity. `services/hub/internal/limits` validates window currency and computes the
projection. Keep `contracts/usage-window-currency-cases.json` in agreement with
keep-warm’s narrower reader.

For a quota decision, `resets_at` must be present and strictly in the future at
the instant of use. The report’s `is_current` is only a hint computed when that
report was produced. A cached percentage or cached true hint cannot make an
expired window current. Unknown reset time remains unknown.

`useUsageReport` shares one report/fetch per backend, refreshes every minute,
and uses a one-second local clock only to redraw deadline/reset guards. It does
not extrapolate the server’s pacing calculation. Backend generations and request
sequence numbers prevent older responses overwriting a newer forced refresh;
failed refresh marks an existing report transport-stale. Settings can force a
refresh after saving the pacing schedule.

`usageReportAttribution` reports match, ambiguous, none, remote, or unavailable.
Claude session paths can identify a root; otherwise a provider with exactly one
report row can be matched. Multiple rows without identity are ambiguous. Peer
sessions cannot borrow a local account report merely because their path or
provider resembles a local default. `SessionAccountUsage` and Overview use this
session-free report rather than requiring a live status line for every gauge.

## Verification

Use the shared pricing, cache-multiplier, model-window, context-health, and
window-currency fixtures for cross-language changes. Account poll/report tests
must exercise zero live sessions, unavailable/expired credentials, explicit
unattributed rows, and elapsed reset times. Renderer usage tests cover attribution,
transport-stale reports, and server-validity deadlines. Run the Rust `analytics`
integration target for persistent history and actual catalog/full ownership;
a passing desktop-only history test does not validate that path. No tests here establish current external provider prices or
live account headroom.
