# Limit-aware agent routing

**Status:** built and wired. The hub reads each provider's remaining
subscription capacity, resolves the work role a caller names into a concrete
`(provider, model, effort)`, and clamps every bus `agents.spawn` to a
per-directory ceiling. There is no GUI for the matrix; the file is the
interface, and this document plus the comments inside the shipped file are the
reference.

Relevant code:

- `services/hub/internal/routing/routing.default.yaml` is the shipped matrix and
  the primary reference for whoever edits the file. It is compiled into the hub
  binary and written to disk verbatim on first run.
- `services/hub/internal/routing/load.go` is the merge, the alias folding and
  the load-time validation.
- `services/hub/internal/routing/service.go` seeds the file, polls it, and holds
  the matrix in force.
- `services/hub/internal/routing/policy.go` reads capacity, decides the routing
  mode, and answers `routing.select`.
- `services/hub/internal/routing/ceiling.go` turns the `ceilings:` block into one
  verdict per spawn.
- `services/hub/internal/routing/decisionlog.go` is the append-only audit trail.
- `services/hub/internal/limits/window.go` is the window-currency rule, which is
  the single most load-bearing thing in the feature.
- `services/hub/cmd/hub/routing.go`, `routingselect.go` and `routingceiling.go`
  are the hub-side wiring: the usage poll, the `routing.select` handler, and the
  spawn gate.
- `services/hub/scripts/routing-limit-harness.mjs` is the runtime harness
  (`make test-routing-harness`).

## What it does

Workflow logic names a **role**. The matrix resolves that role to a
**capability**, and a **profile** resolves the capability to a real provider,
model and reasoning effort. Between those two steps the hub reads claudemon's
`GET /usage/report`, judges how much of each metered provider's allowance is
left right now, and picks a **routing mode** that can move the capability up or
down before it is resolved.

The result is that work moves away from a provider whose allowance is tight, and
that model selection lives in one file rather than in every dispatch site. Model
vendors rename things; the matrix is the only place that has to know.

A routing mode has **three moves**, and they are easy to conflate because two of
them end with a dispatch on a provider nobody named:

| | What moves | Configured by |
|---|---|---|
| **Mode shift** | The ROLE moves to a **different capability** — a conserving scout drops from `balanced` to `cheap`. Which provider that lands on is a consequence, not the aim. | `mode_shifts:` |
| **Effort step** | Nothing moves at all: the **same model** is asked to think one notch less (or more) along its own reasoning ladder. | `mode_shifts.<mode>.effort_step` |
| **Fallover** | The capability stays **exactly the same** and only the **provider** serving it changes, because the primary one cannot be used. | `alternatives:` |

They compose, in that order: the mode arms the effort step and moves the
capability, the ceiling caps the result, the fallover picks who runs it, and the
step is applied last, to the row that actually won. A decision reports the first
through `capability` versus `baseCapability`, and the second through
`fellOverFrom` and a reason sentence naming the primary it passed over.

`capacity` (and the `routing.decision` event's `health`, `pace`, `paceRatio` and
`paceWindow`) is always read for the SUBJECT — the provider step 4 judged the
mode against — because that reading has to exist before either mechanism can
decide whether to move anything. When a mode shift or a fallover lands the
answer on a different provider, `shiftCapacity`/`shiftMode` carry that landing
provider's own reading, and the published `health`/`pace` fields are drawn from
whichever of the two actually describes `provider` (`Decision.EffectiveCapacity`
in `internal/routing/policy.go`). Both readings are kept on the wire rather than
only the second, because "we looked at claude before sending claude the work"
is the claim a decision that never moved is making, and a field that appeared
only after a shift would not support it.

One bus method exposes it, `routing.select`, and it is read-only. Agents holding
the workspacer tools at the operator tier see it as `select_model`. It decides
nothing on its own: a caller passes the answer's provider, model and effort to
`agents.spawn` itself. Nothing calls it on the caller's behalf, so a dispatch
that never asks gets no decision and appears nowhere in the decision log. The
Fleet Manager doctrine is what makes the fleet ask; the desktop spawn dialog and
the web resume paths do not, and are not routed.

## The file

```
<user config dir>/workspacer-hub/routing.yaml
```

On Linux that is `~/.config/workspacer-hub/routing.yaml`, on macOS
`~/Library/Application Support/workspacer-hub/routing.yaml`, on Windows
`%AppData%\workspacer-hub\routing.yaml`. It is a sibling of `jobs.json` in the
hub's own state directory, mode `0600`, and it is refused to `fs.write`. Point
the hub somewhere else with `--routing-file`, or pass `--routing-file ""` to run
on the compiled-in defaults and write nothing.

Four properties are worth knowing before you edit it.

**The shipped defaults are compiled into the binary and written to disk once.**
The bytes that land on disk are the same bytes the binary carries, comments
included, so the file you open explains itself. A seed marker
(`routing.yaml.seeded`) records that the offer was made, so a file you delete on
purpose stays deleted and the hub keeps running on the compiled-in copy.

**Your file is deep merged over those defaults, key by key.** No block is
replaced wholesale. Anything you omit still resolves, and a release that adds a
role, a capability or a profile works against a file written before that name
existed. The merge cannot delete: to take something out of service, write
`enabled: false` rather than removing the entry.

**Saving applies on the next tick.** The service hashes the file's contents
every 30 seconds. A changed threshold, a switched profile or a new ceiling takes
effect without a restart and without any window open, and the spawn gate reads
the same live matrix.

**A file that cannot be read or does not parse changes nothing.** The running
matrix stays exactly as it was and the hub logs why. A half-typed save, or an
editor unlinking the file during its own atomic write, cannot disarm routing.
Every key your file carries that differs from a shipped default is named in the
load log, and so is every key that matches nothing in the defaults, which is
usually a typo.

Model ids are checked at load against the live provider catalogs
(claudemon's `GET /providers/:provider/models` and `claude.listModels`), so an
id your installed CLI no longer serves is reported at load rather than
discovered at spawn time. A provider that cannot be reached is skipped, not
condemned.

## active_profile and the three profiles

`active_profile` names which profile resolves capabilities right now. Three ship.

| Profile | For |
|---|---|
| `mixed` | Both a Codex and a Claude subscription. Codex builds and diagnoses, Anthropic reviews and adjudicates. |
| `codex_only` | One Codex subscription. |
| `anthropic_only` | One Claude subscription. |

`mixed` is the recommended default when both subscriptions exist, for two
reasons. It draws two allowances down in parallel instead of exhausting one, and
it **prefers a different model family for review**: a reviewer from a different
family does not carry the implementer's blind spots. That second reason is the
one that survives even when capacity is not scarce.

That preference is not a guarantee, and the matrix says so rather than letting
you find out. Every capability under `mixed` names **both families** — one as
the primary pairing and the other in an ordered `alternatives:` list — and when
the primary cannot be used the router takes the first alternative that can be,
and names the fallover in the decision's reasons:

```yaml
frontier:
  provider: codex
  model: gpt-5.6-sol
  effort: high
  alternatives:
    - { provider: claude, model: opus, effort: high }
```

A candidate is unusable when its provider's allowance is **RED or EXHAUSTED**,
when that provider's own routing mode is **CONSERVE**, when the provider or the
entry carries **`enabled: false`**, or when the loader **flagged that row** (an
unknown provider id, a model the installed CLI does not serve, an effort it does
not take). Every one of those is judged against the **candidate's own** capacity,
never the primary's: "codex is red, so use claude" is only an argument if
somebody read claude. If nothing in the list is usable, the primary stands and
the answer says the walk was tried and found nothing.

Two consequences worth being explicit about. A codex outage can legitimately
land a `mixed` reviewer back on codex, so `mixed` no longer buys family
diversity by construction — the matrix prefers a different family for review and
says so when it could not. And a caller that pins `provider:` gets **no**
fallover: it asked about that provider, and a model on one it did not ask for is
not an answer to that question. Its alternatives are still read, to find that
provider inside the active profile before borrowing another profile's pairing.

`alternatives:` is a YAML **list**, so the deep merge replaces it wholesale:
mention it for a capability and you own the whole list for that capability. The
primary's own `provider`/`model`/`effort`/`fresh`/`enabled` keys are siblings of
it and stay individually editable. An alternative may not carry alternatives of
its own; the loader reports nesting as an issue.

An alternative's own `fresh:` is **advisory only**. The spawn gate's freshness
check resolves `fresh` through the active profile's **primary** entry for a
capability — it never reads an alternative's row — so what actually governs a
resume refusal after a fallover is the primary's flag, whichever candidate the
walk landed on. The loader flags a mismatch (an alternative whose `fresh:`
disagrees with its primary's) as an issue at load time, because reaching that
asymmetry silently is how a same-family reviewer stops being independent on
the one day the primary is down. The shipped default always agrees on both
sides.

What IS guaranteed, in every profile, always, is freshness. Every profile sets
`fresh: true` on its review capabilities
(`reviewer`, `deep_reviewer`, `frontier_plus`), and in a single-family profile —
or in `mixed` after a fallover — it is the only thing making the reviewer
independent at all. `fresh` means the
worker must **not** inherit the previous agent's conversation: the reviewer gets
the ticket, the acceptance criteria, the diff, the relevant source and the test
results, and does not get the implementer's reasoning history. A reviewer that
inherits the implementer's reasoning is not reviewing it.

`fresh` is reported on the decision (`fresh: true` in the answer) and it is
**enforced**. A bus `agents.spawn` that declares a `role` or a `capability`
whose entry in the active profile carries `fresh: true` may not also carry a
`resumeSessionId`. The hub refuses the call, and the error names the session the
spawn would have inherited. It refuses rather than clamping because there is no
weaker value of "continue that session": dropping the field would start a new
session while the caller went on believing it had continued one, which is the
quieter failure of the two.

Three things follow from how the check reads a spawn.

- A spawn that declares no role and no capability makes no freshness claim, so
  it resumes normally. A role or capability the matrix does not name is the same
  answer, because the matrix expresses no opinion about it.
- A role is read at its strongest. The resume is refused if the capability under
  `roles:` is fresh, or if any capability a `mode_shifts:` entry would move that
  role onto is fresh. The gate cannot know which mode the caller decided under,
  and being wrong in this direction costs only continuity.
- The active profile decides. Writing `fresh: false` on its reviewer entry is
  honoured, and a flag in a profile you have not selected refuses nothing.

Which callers can reach the rule today, plainly. The MCP facade's `spawn_agent`
is the only wire that carries `role` and `capability`, and it has no
`resumeSessionId` field at all, so a Fleet Manager cannot ask for a resume and
can never be refused one. The wires that do carry `resumeSessionId` are the
desktop provider's `agents.spawn` (reached by the web `/app` and `/m` clients
over the bus) and the renderer's resume-a-recent-session path, and none of them
declares a role or a capability, which the check correctly reads as no freshness
claim. So the refusal is live code on a path nothing currently walks. It starts
biting when a caller both declares a review role and asks to resume, which is
what the Fleet Manager doctrine now teaches it never to do.

The rule is keyed off what the spawn declares rather than off the `decisionId`
it quotes, which is safe here in a way it would not be for the ceiling. Refusing
a resume only ever gives a caller less, so a caller who lies about its role gains
nothing it could not have had by declaring no role at all. The same rule binds on
a federated spawn: enforcement lives in `sanitizeSpawnParams`, the one spawn-path
function both the local and the federated call paths share, so the peer applies
it to what arrives.

Choosing what to hand a fresh worker instead still belongs to whatever dispatches
the review: the ticket, the acceptance criteria, the diff, the relevant source
and the test results.

## The blocks

Read `routing.default.yaml` for the per-key detail. This is what each block is
for and when you would touch it.

**`capabilities:`** is the vocabulary workflow logic may use: `cheap`,
`balanced`, `frontier`, `frontier_max`, `reviewer`, `deep_reviewer`,
`frontier_plus`. Adding a name here does nothing on its own, because every
profile then has to resolve it. Edit it if you are introducing a genuinely new
tier of work.

**`capability_ranks:`** says how strong each capability is. The list above is a
vocabulary and its order is documentation, not a ladder: `reviewer` is listed
after `frontier` and is cheaper than it. Only the `ceilings:` block reads these
ranks, and a capability with no rank cannot be compared, so a ceiling clamps it
rather than waving it through. Edit this whenever you add a capability.

**`roles:`** maps each kind of worker to a capability: `supervisor`, `scout`,
`mechanical`, `implementer`, `reviewer`, `deep_reviewer`, `fixer`,
`complex_fixer`, `validator`, `diagnostician`, `judge`. These names are the
vocabulary `routing.select` and the `select_model` tool accept. Edit it to change
what a kind of work is worth, without touching any model name.

`roles.supervisor` is not consulted yet. The Fleet Manager's own model is chosen
before there is a manager to ask, from Settings, Fleet Manager
(`agents.managerProvider`, `managerModels`, `managerEfforts`). The row exists so
the vocabulary is complete.

**`profiles:`** resolves each capability to `{ provider, model, effort,
min_effort, fresh, enabled, alternatives }`. `provider` is a workspacer provider id (`claude`, `codex`,
`copilot`, `opencode`, `pi`); `openai` and `anthropic` are accepted as aliases
and folded at load. Effort ladders differ per provider and are not
interchangeable: Claude takes `low|medium|high|xhigh|max`, Codex takes
`minimal|low|medium|high|xhigh`. `min_effort` is the floor a mode's effort step
may not push that row below (see *Effort stepping*), and `alternatives:` is the
ordered list of other pairings that serve the same capability when the primary
cannot be used. Edit this when a vendor renames a model or you want a different
model behind an existing capability.

**`providers:`** describes what is known about each provider's capacity, not what
it can do. `metered: true` means the provider publishes an allowance worth
routing around; only `claude` and `codex` do. `when_unknown` is the health to
assume when nothing trustworthy can be read: `yellow` is the conservative answer
for a metered provider, `unmetered` means the question does not apply.
`enabled: false` here takes a provider out of service entirely, and routing
refuses rather than substituting another one. Edit it if you add a provider or
want one held out.

**`thresholds:`** holds the numbers that pick a routing mode, as configuration
rather than as constants in Go.

- `health.yellow_at_used_pct: 70` and `health.red_at_used_pct: 90` are the bands
  a used-percentage is folded into. Health is taken from the **worst** applicable
  bucket, so a provider is not green because its five-hour window is healthy
  while its weekly one is not. Only red and exhausted force `conserve`; yellow on
  its own does not, and it does block `spend_down`.
- `spend_down.time_to_reset_minutes: 90`, `min_remaining_pct: 50` and
  `max_forecast_pct_of_remaining: 30` are the three arms of the spend-down rule,
  all of which must hold. Unused subscription capacity is worth nothing after the
  reset, so near one, with capacity left and little demand coming, it buys
  confidence instead of expiring.
- `pacing:` is the same allowance judged against the clock rather than as a
  level. It has its own section below.

**`forecast_weights:`** weights how much premium demand a ticket in each phase
implies: `scouting: 1`, `implementation: 4`, `review: 2`, `fixing: 2`,
`validation: 1`. Be clear about what these do today. A caller that supplies
`expectedWork` phase counts gets them weighted, and the answer reports the
arithmetic (`demand.units`, `demand.phases`), but weighted units do **not**
become a percentage of an allowance, because there is no cost model behind that
conversion yet. A work-only forecast therefore leaves demand UNKNOWN for the mode
rules, which cannot promote and cannot conserve on it. The only form the mode
rules act on is `forecastDemandBeforeResetPct`, a share the caller measured or
asserts. Editing the weights changes the reported arithmetic, not the mode.

**`modes:`** is the manual override: `auto | conserve | normal | spend_down`,
globally and per provider. A per-provider entry wins over the global one, except
`auto`, which is a deferral rather than a verdict and falls through to the
global. To hold one provider out of a global `conserve`, say `normal`, which is a
verdict and wins. Edit it when you want to conserve a provider for a few hours
regardless of what the thresholds think.

**`mode_shifts:`** is what a mode does, keyed by mode and then by role. `normal`
has no block, because the `roles:` table already is the normal answer. A role
with no entry under a mode keeps its usual capability, and that absence is
meaningful: under `conserve` the scout drops to `cheap` while the fixer stays on
`balanced`, because demoting the role that repairs broken work is the demotion
most likely to cost more than it saves. Edit it to change how aggressively a mode
moves work.

A shift is allowed to cross providers, which is the point of the feature. When it
does, the landing provider's own capacity is read and judged before the move is
applied, and the move is refused if that provider is itself conserving. Moving
work onto a constrained provider because a different one was constrained is worse
than not moving it.

Each mode block also carries `effort_step` and `effort_step_capabilities`, which
are the mode's *other* lever — the same model, thinking less. They have their own
section below, as does the per-row `min_effort` floor that bounds them.

**`ceilings:`** is the per-directory cap. It has its own section below.

Three request fields are accepted and not yet acted on: `difficulty`, `risk` and
`decisionDensity`. They are the caller's own classification, and deriving a
capability from them is a model call rather than a threshold, so the matrix does
not use them today.

## The window-currency rule

This is the heart of the feature, and it is one sentence:

> A usage reading is used only when its reset time is in the future **at the
> moment of the decision**. A window that has already reset yields UNKNOWN, never
> a percentage, never a remaining capacity, and never a time-to-reset.

The reason is that a stale reading is wrong in two directions at once, and both
of them are expensive.

A window that closed two days ago at 67% used still reports 67% used. Believed,
that makes a provider with a completely fresh allowance look two-thirds spent, so
the router conserves premium capability nobody needed to conserve. At the same
time the reset time it carries is in the past, and a rule written as
`time_to_reset < 90 minutes` is trivially true of a negative number, so the same
stale row also looks like a window about to roll over. That is the spend-down arm
firing against an allowance whose reset already happened.

The guard is structural rather than a sign check somebody has to remember. The
only time-to-reset available to the policy layer is
`limits.BucketReport.ResetsInSeconds`, which is populated solely from
`Reading.TimeToReset`, and that accessor is reachable only on a current window,
so it cannot return a non-positive duration. There is no comparison in
`policy.go` that a stale reading can reach.

Two consequences to expect when reading an answer:

- **UNKNOWN is a real answer and does not quietly become healthy.** Three of the
  five providers have no readable quota at all: Copilot answers 403 to the only
  endpoint that would say, and OpenCode and Pi never appear in the usage document.
  What to do about an unknown provider is read from `providers[].when_unknown`
  rather than assumed, which is why a decision reports the **observed** health and
  the **assumed** health as two separate fields. Collapsing them is how "we could
  not read Codex" becomes "Codex is fine" three layers downstream.
- **Document staleness and window currency are different questions.** The hub may
  hold a two-second-old usage document that contains a window which lapsed two
  days ago. The document is re-judged against the caller's clock on every
  decision, so nothing caches a verdict.

The usage poller is dormant until something asks for a routing decision, and it
winds down again 15 minutes after the last ask. An ordinary install that never
consults routing takes no readings at all.

## Pacing: the allowance against the clock

`health:` answers **how much** of an allowance is gone. It cannot answer whether
that is a lot for a Tuesday morning, and it is the same 40% that is comfortable
six days into a seven-day window and is a fleet about to run dry six hours into
one. Pacing supplies the missing term.

The arithmetic is one division:

```
elapsed  = (window_length - time_to_reset) / window_length
expected = the curve's share of the allowance that should be gone by now
ratio    = (used_percent / 100) / expected
```

`window_length` is what makes this possible and is the piece that was missing.
Codex has always reported one per window (300 and 10080 minutes); claudemon now
reports the two Anthropic lengths as well, because the window names
(`five_hour`, `seven_day`) assert them and the OAuth endpoint never states them.
The **monthly** overage window deliberately keeps no length: a calendar month is
not a fixed number of minutes and the endpoint does not say which month a
reading belongs to, so it is never paced.

Everything above is read through the same currency guard as everything else. A
window that has rolled over has no length, no reset and no percentage available
to it, so it cannot be paced at all — the pace ratio is one more thing a stale
reading must not be able to reach, whether "stale" means the window itself has
closed or the daemon's own reading of a still-open window is old. A window can
be current — resets_at strictly in the future — while its account row carries
`fresh: false`; a reading like that is judged exactly as a rolled-over one is,
PaceUnknown, so an old observation cannot license a spend-down or a conserve
either. `health:` does not consult the same flag today, so a stale reading can
still set a provider's health while leaving its pace unknown — the two answers
are allowed to disagree, and the pace explanation says "stale" by name so the
disagreement is never silent.

### What pace may do

- It may **add** `conserve` to a provider whose window is being drained faster
  than it refills. That is the case `health:` cannot see: 80% of a five-hour
  window gone with half the window left is only YELLOW on the ladder.
- It may **block** a `spend_down` without conserving. Spend-down converts
  allowance that would expire unused into confidence, and allowance already
  running ahead of the curve is not going to expire unused.
- It may **never** override RED or EXHAUSTED health. The pace arms sit after the
  health arms, so a flattering ratio can never talk a nearly-spent allowance
  back down to normal.
- It **never promotes anything.** There is no pace state that unlocks a stronger
  model.

For a provider with more than one readable window, the **worse** (highest) ratio
binds, and the answer names which window it came from. That is what makes an
Anthropic decision "the worse of the five-hour and the seven-day pace" with no
per-provider rule anywhere: both Anthropic windows carry a length now, so both
are judged. Codex's five-hour reading is frequently stale, so in practice its
seven-day pace is what answers for it — by the currency guard, not by a special
case. A provider whose windows carry no length at all (Copilot, anything absent
from the report) is UNKNOWN, which conserves nothing and unlocks nothing.

### The knobs

```yaml
thresholds:
  pacing:
    enabled: true
    conserve_at_ratio: 1.25
    block_spend_down_at_ratio: 1.0
    bootstrap:
      min_elapsed_pct: 5
      expected_offset_pct: 2
    seven_day:
      curve: calendar
      timezone: local
      weekend_weight: 0.5
      weekend: spend_tail
      weekend_reserve_pct: 0
```

**`enabled: false` reproduces the pre-pacing answers exactly**, including the
absence of the `pace` fields on the answer and on the `routing.decision` event.
It is the switch to reach for if a fleet's rhythm does not fit a curve.

**The bands** are ratios of consumed-to-expected; 1.0 is exactly on the curve.
`block_spend_down_at_ratio` is never above `conserve_at_ratio` — a band that
conserved without blocking spend-down would be asking for two modes at once, and
the file is reported as an issue if it says so.

**The bootstrap block** is where this arithmetic would otherwise lie. One
percent used against a fifth of a percent elapsed is a ratio of five, so
`min_elapsed_pct` refuses to take a verdict at all in the first stretch of a
window (pace is UNKNOWN there, which conserves nothing), and
`expected_offset_pct` widens the denominator by that many percentage points so
an ordinary opening burst does not divide by nearly zero.

**The seven-day curve** applies to the weekly window only; a five-hour window
has no weekday shape to have an opinion about.

- `curve: calendar` is linear in wall-clock time and is what ships. A fleet that
  works at the weekend would otherwise be told to conserve on Saturday for no
  reason.
- `curve: workdays` budgets a weekend hour at `weekend_weight` of a weekday
  hour, so a week's allowance spent across five working days is **on plan**
  instead of 40% over it. The weight must be strictly positive — at zero, a
  window that ends over a weekend has no expected progress at all and every
  weekend hour reads as infinite overspend — and the load-time validation says
  so, while the arithmetic falls back to the calendar curve and states the
  fallback in the answer rather than dividing by zero.
- `timezone: local` is the host's own zone. A weekend is a local fact and a
  curve computed in UTC for a fleet in UTC+13 is wrong by most of a day. Any
  IANA name works; one this host's tzdata cannot resolve is reported and falls
  back to the calendar curve.
- `weekend: spend_tail` holds nothing back: whatever is left when the working
  week ends may be spent over the weekend. `weekend: reserve` keeps
  `weekend_reserve_pct` of the allowance against the curve, which makes the
  pacer start saying "overspending" earlier during the week. A reserve written
  under `spend_tail` is ignored, and both the load-time issue and the answer's
  own explanation say so, rather than leaving a number in the file that looks
  like it does something.
- `weekend: reserve` is applied to the seven-day window's expected share
  regardless of which curve produced it — `curve: calendar` included, not only
  the `workdays` curve or its unusable-workdays fallback. A reserve tightens
  the expected-progress line either way, so `weekend: reserve` under
  `curve: calendar` starts saying "overspending" earlier during the week too.

### What pacing does not do yet

Claude's stream transport publishes a rate-limit **status** as well as a
utilization: `status: allowed_warning` on a `rate_limit_event`, which is the
provider's own "you are close to this limit". It is not folded into pace, and
that is deliberate rather than an oversight. The warning arrives session-scoped
and latest-wins (`AgentUpdate::RateLimitStatus`), carrying a rendered sentence
and neither the window it describes nor that window's reset time, and
`/usage/report` does not carry it at all. Associating it with a
currency-guarded window would mean retaining the event's `rateLimitType` and
`resetsAt` through the account store and publishing it per window in the report
— a claudemon data-model change, not a routing one. Attaching it to a window on
a guess is the exact shape of the bug this layer exists to refuse, so it stays
a follow-up.

### Reading a paced answer

`routing.select` returns `capacity.pace` (the window that bound, its ratio, the
terms of the division and a sentence) and `capacity.paceWindows` (every window,
so you can see what the others said). The `reason` list carries the same
sentence in prose, and the `routing.decision` event carries `pace`, `paceRatio`
and `paceWindow` so a fleet display can caption a mode change without reading
the reasons. All of those are absent when pacing is off.

## Effort stepping: the same model, thinking less

A capability shift is a blunt instrument. It changes which model runs the work,
so conserving means a scout stops being a Sol scout and becomes a Luna one.
Between "the same model" and "a different model" there is a move the matrix
could not previously make: **the same model, at a lower reasoning effort**.

```yaml
mode_shifts:
  conserve:
    scout: cheap
    effort_step: -1
    effort_step_capabilities: [frontier, frontier_max, deep_reviewer, frontier_plus]
  spend_down:
    implementer: frontier_max
    effort_step: 1
    effort_step_capabilities: [frontier, frontier_max, deep_reviewer, frontier_plus]

profiles:
  mixed:
    deep_reviewer:
      provider: claude
      model: opus
      effort: high
      min_effort: high      # an effort step may never trim this row below `high`
```

### `effort_step` — a notch count, not a level

`effort_step` is a number of rungs on the **provider's own** ladder, applied to
the row the answer landed on. `-1` steps down one, `+1` steps up one, and `0`
(or the key absent) reproduces the pre-stepping answer exactly, reason list
included.

It is a count rather than a level name because **the ladders are not portable**:
claude runs `low, medium, high, xhigh, max`, codex stops at `xhigh`, and copilot
starts below both at `none`. One notch down means the same thing on all three;
`medium` does not. The ladders live in `internal/routing/effort.go`, taken from
the adapters that build each CLI's argv.

Three things stop a step:

- **A row with no `effort:`** is not stepped. It runs at the provider's own
  default, which this layer does not know — `frontier_plus: {provider: claude,
  model: fable}` has no rung to count from — so the answer says so instead of
  inventing a level nobody wrote.
- **A provider with no published ladder** (`opencode`, `pi` — BYO key) is not
  stepped, for the same reason.
- **The ends of the ladder.** A step through the floor or the ceiling clamps
  and says which, because "we tried and could not" is a different fact from "we
  did not try".

Under `spend_down` there is one further rule: **one promotion per decision**. If
the mode has already moved the role UP a capability tier, the effort is left at
what that tier's own row declares rather than being raised a second time. The
comparison is on `capability_ranks:`, not on whether a shift fired, because the
ceiling runs between the two and routinely takes the promotion back — under the
shipped `default: {max_capability: frontier}` a spend-down implementer is moved
to `frontier_max` and clamped straight back, and nothing was promoted there.

### `effort_step_capabilities` — where trimming is worth it

Empty means every capability, which is **not** what ships. The default list is
`frontier`, `frontier_max`, `deep_reviewer`, `frontier_plus`: the tiers where
reasoning time is the expensive part. A scout on Sonnet at `medium` is mostly a
worse scout rather than a cheaper one, so `cheap` and `balanced` are left alone.
A capability outside the list is reported as untouched, with the reason, rather
than silently skipped.

### `min_effort` — the floor a row will not go below

A per-row floor, validated at load against that provider's ladder. The shipped
`mixed` and `anthropic_only` profiles set `min_effort: high` on `reviewer`,
`deep_reviewer` and `frontier_plus`: a review that is trimmed is not a review.

It is honoured on **alternatives** as well as primaries, and the shipped file
writes it on both — a fallover takes the alternative's own row, so a floor
written only on the primary stops binding at the exact moment the answer moves.

### When a step fires

Two bands, and the order inside `Select` is the feature rather than an
implementation detail:

| Evidence | What happens |
|---|---|
| `conserve` / `spend_down` | that mode's own `effort_step`, alongside the capability shift it already performs |
| `normal`, pace in the **lower** overspend band (at or past `block_spend_down_at_ratio`, below `conserve_at_ratio`) | conserve's step **only** — the capability is not moved |

The lower band already blocked a spend-down and did nothing else. It now also
trims one notch: being slightly ahead of the curve is a reason to spend a little
less, not a reason to change which model does the work. The step is armed from
the SUBJECT provider's reading — the same one the mode came from — and applied
last, to whichever row the shift, the ceiling and the fallover walk finally
chose. After a fallover that is the alternative's own effort and its own
`min_effort`.

### What it does not fix

**An effort step trims thinking tokens, not context re-reads.** A worker at
`medium` still reads the same files, still re-reads them after a compaction, and
still carries the same system prompt and tool definitions on every turn. So
stepping degrades gracefully — it buys a real but modest saving for a modest
loss of depth — and it **will not rescue a red window**. The tier shift is still
the tool that bends the curve, and a provider that is actually out of allowance
is routed away from rather than asked to think less.

### Reading a stepped answer

`routing.select` returns `effortStep: {from, to, why}` whenever a step was
ARMED, including when it was armed and then clamped, floored, or refused — the
pair being equal is the answer to "we looked and did not move". The same
sentence is in the `reason` list, the field rides the `routing.decision` event,
and the whole decision (this field included) is written to the decision log.
Nothing is present when no step was armed.

## Live provider availability

The fallover triggers in `alternatives:` were all facts about the **document** —
a row switched off, a load-time issue, a health reading. One was missing, and it
is the one that bites first on a new machine: **the provider's CLI is not
installed**, so nothing can be launched there at all, however healthy its
allowance looks.

The hub already boots each provider's CLI to fetch its model catalog, which is
what `routing.yaml`'s model ids are validated against. That probe's answer is
folded into a small map and **injected into `Select` as an argument**, alongside
the usage snapshot and the clock. `Select` stays pure: it does no I/O, so a fact
about the world outside arrives from the caller or does not arrive at all.

There are **three** states, and the third one is the whole safety argument:

| | Meaning | Effect on routing |
|---|---|---|
| available | the provider answered with models it can launch | usable |
| unavailable | the **provider itself** answered and can launch nothing — the CLI is not installed, or cannot start | unusable; the walk moves on and the reason names it |
| unknown | nobody could ask (claudemon down, no peer to answer `claude.listModels`, never probed) | **used as normal** — exactly as before this existed |

Collapsing unknown into unavailable would mean a hub that cannot reach claudemon
for thirty seconds declares every provider dead and routes nowhere. That is
strictly worse than routing to a provider that turns out to be missing: the
second failure is loud, immediate and recoverable, and the first looks like the
router breaking for no reason.

Availability is about the PROVIDER. A candidate on a provider that is up, whose
own **model** the loader flagged (`ValidateAgainstCatalog` — "codex does not
serve model X"), is still unusable, and the answer quotes that reason instead.

The map is refreshed on the same cadence as the catalog it comes from, and the
refresh is kicked by a decision rather than by a timer: it runs in the
background, at most one at a time, and no more often than every few seconds, so
nothing boots a provider CLI on a machine where nobody is routing. A decision
acts on the last probe and the next one acts on this one; the first decision
after a hub start therefore routes with an empty map, which is the fail-open
state.

## Ceilings

A ceiling is the most a spawn started in a given project directory may be given,
whatever the caller asks for. Stated plainly: **it is what a Fleet Manager may
not raise for itself.**

```yaml
ceilings:
  default: { max_capability: frontier, max_tool_scope: operator }
  /home/you/Work/some-client-repo: { max_capability: balanced, max_tool_scope: triage }
```

- `max_capability` is the highest capability a spawn there may resolve to.
- `max_tool_scope` is the highest authority tier a worker there may hold:
  `view`, `triage` or `operator`. ("Tier" already means authority in this
  codebase, which is why the model axis is called capability everywhere else.)

Keys are absolute directories. An exact match wins, then the **longest matching
ancestor**, then `default`, so one entry covers a whole tree. The match is
lexical over an already-resolved path, and the enforcement sites canonicalize the
spawn's cwd first with the same walk the filesystem guard uses. A symlink into a
capped tree therefore does not walk around its ceiling, and a cwd that cannot be
resolved gets the `default` entry rather than no entry.

The clamp is applied in `sanitizeSpawnParams`, the one piece of spawn-path code
in this repo that is not a twin, and because `methodSanitizers` is the single
dispatch table for both direct and federated calls, it covers the federated hop
too. Every `agents.spawn` that arrives over the bus passes it: the remote
clients, the MCP facade, a hub job, a federated peer. A spawn asking for more is
not refused outright. The excess is clamped, the caller is told what was taken in
the spawn's own answer (`escalationScrubbed`), and the replacement model is
named rather than deleted, because an omitted model is not a weak model, it is
whatever the provider defaults to below where any ceiling can see.

`routing.select` applies the same ceiling through the same function before it
answers, so it cannot advise a model the gate would then take away. The gate
still clamps independently, because it is the security boundary and must refuse a
caller that ignored routing entirely.

`max_capability` binds three things: the spawn's declared `capability`; the
capability the **model** it names resolves to in the profiles, read at its
strongest (`opus` with no effort could be `frontier`, `deep_reviewer` or
`frontier_max`, so under a `frontier` ceiling it is refused, and naming
`effort: high` narrows the reading and admits it); and the model the clamp leaves
behind. A model the matrix never mentions is not judged at all.

A **context-window suffix is not another model**. `opus[1m]` is `opus` with a 1M
window instead of the standard 200K, so it is judged as `opus`, and either
spelling in `profiles:` matches the other. That mattered more than it looks:
`opus[1m]` is the desktop's shipped `claude.defaultModel`, so it was what a spawn
got by leaving `model` out entirely, and while the two strings had to match
exactly it was the one Claude model no ceiling could read. The suffix is
normalized for the **comparison only**. A spawn the ceiling admits reaches the
provider spelled exactly as it was sent, window request included. A spawn the
ceiling clamps is answered with the permitted capability's profile entry
verbatim, so the replacement runs the window **its own** entry asks for rather
than inheriting the refused model's; if that drops a `[1m]`, the refusal says
so.

A ceiling value the file cannot read **denies** the spawn rather than being
skipped: a `max_capability` that `capability_ranks:` does not rank, or a
`max_tool_scope` that is not one of the three tiers. A typo in a policy file must
not be the quietest possible way to delete the policy. An omitted key is
different and still means "this row does not cap that axis".

Two honest limits. The local desktop spawn dialog is deliberately not clamped,
because that is a human at the machine clicking Spawn. And a ceiling is enforced
exactly as strongly as the Fleet Manager's own permission mode: it closes every
capability door, so with approvals on an edit to the file raises a prompt you
see, and with full access on it does not. A ceiling is not a sandbox.

### What the shipped default does and does not cap

`default: { max_capability: frontier, max_tool_scope: operator }`.

It caps the **model axis at `frontier`** for every directory with no entry of its
own. `frontier_plus` is refused everywhere by default, and so is `frontier_max`,
which matters most under `spend_down`: that mode promotes `implementer`,
`complex_fixer` and `diagnostician` to `frontier_max`, and those promotions land
back on `frontier` under this default. Everything at or below `frontier` is
untouched, so ordinary implementation and review work is unaffected.

It caps **no authority at all**, because `operator` is the top tier. Lower it
here or per directory if a worker in a given tree should never be able to
approve, spawn or write.

## What a judge resolves to

This is the first surprising thing most people hit, so it is worth stating
before you meet it.

`roles.judge` is `frontier_plus`, the reserved escalation tier, which the
shipped `mixed` and `anthropic_only` profiles resolve to Claude's Fable. The
shipped `default` ceiling caps capability at `frontier`. Since `routing.select`
is ceiling-aware, it returns the capped answer rather than advising something the
gate would refuse.

So under the shipped default, a `judge` dispatch resolves to **capability
`frontier`, which under `mixed` is `codex` `gpt-5.6-sol` at `high` effort**. The
answer still records `baseCapability: frontier_plus`, reports the ceiling it was
resolved under, and says so in its reason list, so the cap is visible rather than
an unexplained downgrade.

To get Fable for the judge, raise the ceiling. Either globally:

```yaml
ceilings:
  default: { max_capability: frontier_plus, max_tool_scope: operator }
```

or, better, for the one tree where you want it:

```yaml
ceilings:
  /home/you/Work/critical-service: { max_capability: frontier_plus }
```

Save the file and the next tick applies it, to both `routing.select` and the
spawn gate.

## The decision log

`routing-decisions.jsonl` sits beside `routing.yaml`, mode `0600`, append only.
It has two row kinds joined by a `decisionId`:

- `kind: "decision"` is one answer from `routing.select`: the role, the base and
  final capability, the provider, model and effort, the mode, the capacity
  picture it was judged against, the ceiling it was resolved under, and the
  reason list.
- `kind: "spawn"` is one `agents.spawn` as the gate saw it: the role and
  capability declared, the canonical cwd the ceiling was looked up on, the
  provider, model, effort and tool scope, the caller's tier and credential
  fingerprint (never the token), the ceiling verdict, and what was scrubbed.

It exists for three things. **Audit**: a routing decision commits an hour of a
frontier model's allowance, and a clamp that silently took a model away would be
a downgrade only a server log knew about. **The join**: `decisionId` is what makes
the decision and the worker it produced one row instead of two guesses, including
on a headless node, which writes no analytics at all. **Calibration**:
`forecast_weights` are unitless today, and the honest way to give them units is
measurement, which needs a record of what was decided next to what it cost.

A spawn's first message and its `profileId` are deliberately not in it. Neither
is the caller's cwd in the `routing.decision` bus event, which is open by
decision and reaches a view-tier phone; the directory stays in the 0600 file.

The file rotates rather than being trimmed: past 8 MiB the live file is renamed
to `routing-decisions.jsonl.1` and a fresh one starts, so two generations are
kept. Nothing about the log is fatal. A full disk or an unwritable directory
complains once and routing keeps deciding.

## Checking that it works

**Watch the log fill.** Nothing consults routing on an ordinary desktop install,
so an empty file means nobody has asked, not that the feature is broken. Ask for
a decision (an operator-tier agent calling `select_model` is the easy way), then:

```sh
tail -n 2 ~/.config/workspacer-hub/routing-decisions.jsonl | python3 -m json.tool
```

A `decision` row appears immediately. A `spawn` row appears when something
actually spawns, and if it quoted the decision, the two carry the same
`decisionId`.

**Read the load log.** On boot and on every save the hub prints which file it
loaded, the active profile, and each key of yours that changes a shipped default.
Keys matching nothing in the defaults are printed separately, which is where a
misspelled parent block shows up.

**Run the harness.**

```sh
make test-routing-harness
```

It starts a real hub against a fake claudemon that serves usage states you cannot
reproduce on demand: a stale Codex window, a window resetting exactly now, a
Copilot 403, providers absent from the document entirely, and two **pacing**
states that are indistinguishable from a healthy one by used-percentage alone.
It asserts that stale readings come back UNKNOWN rather than choosing a mode,
that the Anthropic window lengths arrive end to end and produce a real pace
ratio, that an over-curve window conserves while a YELLOW health alone would
not, that a provider running ahead of the curve blocks a spend-down without
conserving, that the `routing.decision` event carries the pace, that a provider
publishing no window stays pace-UNKNOWN, that the decision and its spawn both
reach the log, that a capped directory caps `routing.select` and the spawn gate
identically, and that a symlink into that directory does not walk around either.

It also runs the fallover and stepping shapes end to end: a RED primary landing
on the other family with the event's `health` and `fellOverFrom` describing the
provider that will actually run the work, a disabled middle alternative that the
walk steps over and NAMES, a pinned provider served from its own profile without
borrowing another's pairing, an independent-family preference honoured and then
honestly reported as lost when capacity will not allow it, conserve trimming a
frontier row's effort while leaving a balanced one alone, spend_down stepping up
and stopping at both the ladder ceiling and the one-promotion rule, a
cross-provider mode shift with a fallover walk running on top of it, and a
provider whose catalog reports no launchable model being routed around while an
unprobeable one is not. `ROUTING_HARNESS_REQUIRE_ROUTING=1` makes a parked
assertion a failure rather than a note.

The Go unit tests cover the merge, the validation, the mode rules and the ceiling
arms:

```sh
cd services/hub && go test ./internal/routing/... ./internal/limits/... ./cmd/hub/...
```

## Why there is no GUI

Routing exposes no write RPC over the bus and will not get one. `config.save`
cannot address another file, `library.save` is confined to the library, and
`fs.write` refuses the hub's state directory. That is the whole reason the
`ceilings:` block is a ceiling rather than a suggestion: an operator-tier Fleet
Manager can read a routing decision and cannot raise its own cap. Editing the
file is a deliberate act by a person with a text editor, which is the property
being protected.
