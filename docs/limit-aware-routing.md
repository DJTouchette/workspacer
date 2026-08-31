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
it buys model-family diversity on review: a reviewer from a different family
does not carry the implementer's blind spots. That second reason is the one that
survives even when capacity is not scarce.

A single-family profile cannot buy that diversity, so it compensates with
freshness. Every profile sets `fresh: true` on its review capabilities
(`reviewer`, `deep_reviewer`, `frontier_plus`), and in a single-family profile it
is the only thing making the reviewer independent at all. `fresh` means the
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

**`profiles:`** resolves each capability to `{ provider, model, effort, fresh,
enabled }`. `provider` is a workspacer provider id (`claude`, `codex`,
`copilot`, `opencode`, `pi`); `openai` and `anthropic` are accepted as aliases
and folded at load. Effort ladders differ per provider and are not
interchangeable: Claude takes `low|medium|high|xhigh|max`, Codex takes
`minimal|low|medium|high|xhigh`. Edit this when a vendor renames a model or you
want a different model behind an existing capability.

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
Copilot 403, providers absent from the document entirely. It asserts that stale
readings come back UNKNOWN rather than choosing a mode, that the decision and its
spawn both reach the log, that a capped directory caps `routing.select` and the
spawn gate identically, and that a symlink into that directory does not walk
around either. `ROUTING_HARNESS_REQUIRE_ROUTING=1` makes a parked assertion a
failure rather than a note.

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
