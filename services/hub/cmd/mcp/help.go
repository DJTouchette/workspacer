// The help tool: on-demand usage docs for the facade's tools.
//
// Tool descriptions are deliberately one line each — every connected agent
// pays context for the schemas whether or not it uses them — so the guidance
// that used to live in hand-written system prompts lives here instead, fetched
// only by agents that actually go to use the tools. The tool list itself is
// rendered from the build registry, so help can never describe a tool the
// caller's tier doesn't hold.
package main

import (
	"context"
	"fmt"
	"sort"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type helpIn struct {
	Topic string `json:"topic,omitempty" jsonschema:"a tool group to expand (one of the groups the overview lists); omit for the overview"`
}

// groupGuidance is the per-group usage text — the "how", where the tool
// descriptions carry only the "what". Written for the model.
var groupGuidance = map[string]string{
	"observe": strings.TrimSpace(`
Start with list_agents (cheap fleet overview), then go deeper on ONE session.
The fleet may span machines: when hubs are federated, list_agents and
list_snapshots include every connected peer hub's sessions, and each remote
row carries a "hub" field naming its machine (local rows have none). That hub
value must round-trip: pass it as the hub input on the per-session tools
(get_snapshot, get_conversation, get_transcript, send_message, approve,
answer, signal, set_approval_gate) to reach the remote session; omit it for
local sessions.
Reading another agent's activity, cheapest first:
- get_conversation with lastMessage:true: returns { seq, lastMessage } — just
  the session's final assistant message. The right way to read a finished
  worker's report without paying for its whole conversation.
- get_conversation with textOnly:true: only the user/assistant text turns
  (tool calls, tool results, and usage blobs stripped) — the dialogue without
  the machinery. Both reductions compose with sinceSeq; lastMessage wins when
  both are set.
- get_conversation with sinceSeq: track the returned seq per session and pass
  it back next time, so you only ever digest new turns. This is the right tool
  for polling "what has agent X done since I last looked".
- get_snapshot: full live detail for one session (turns, tools, usage, pending
  approval/question). Heavier; use when you need everything at once.
- get_transcript: the raw transcript. Largest payload — prefer the two above,
  or spawn a cheap worker (spawn_agent with toolScope "view") whose whole job
  is to read it and reply with a digest, so it never enters your own context.
When you reference a session in an answer, write its id as session:<sessionId>
so the UI renders a clickable link.`),
	"spawn": strings.TrimSpace(`
spawn_agent starts a new coding-agent session and returns its sessionId.
- Pass message: the TASK ITSELF, as the new agent's first turn. Dispatching is
  one call — you do not spawn, wait for the id, and then send_message. The host
  hands the prompt to the daemon as part of the spawn, so it cannot race the
  agent coming up. Host contracts land through a separate instruction channel,
  not inside this user-visible task, and reach the agent's first turn even when
  message is omitted. Use send_message for anything AFTER that first turn.
- Pass label (short human name) and parentSessionId (your own session id) so
  the new agent nests under you in the UI. That parent metadata, together with
  the manager flag, is also what identifies a real fleet worker; ordinary panes,
  managers, tours, and unmanaged sessions do not receive worker contracts.
- Give the new agent workspacer tools only when it needs them, at the LOWEST
  tier that works: toolScope "view" for summarizer/reader workers, "triage" to
  also approve/reply/interrupt, "operator" for everything (spawning included).
  mcpFacade:true is the legacy spelling of operator.
- skipPermissions omitted = bypass if your session carries the full-access
  grant (the operator turned on full access for the fleet, which
  means the agents you dispatch skip approvals), else the workspacer config
  default (the same one the desktop spawn dialog pre-selects:
  claude.skipPermissionsDefault, or a bypass defaultPermissionMode). An
  explicit true/false always wins — pass false to dispatch ONE worker with
  approvals on. Requested or defaulted, a bypass is honored only when your
  session token carries the grant — ungranted spawns start with approvals on.
- resultSchema (optional) asks the worker for a MACHINE-READABLE report as well
  as its prose: pass a JSON Schema and the worker is told to end its final
  message with a fenced wks-result block matching it, which arrives back on your
  worker-finished wake already parsed and VALIDATED. Use it whenever you will
  transcribe the outcome into a brief — it turns restating a report into copying
  fields. Typical shape: {"type":"object","required":["commit"],"properties":
  {"commit":{"type":"string"},"filesChanged":{"type":"array","items":
  {"type":"string"}},"checksRun":{"type":"array","items":{"type":"string"}},
  "caveats":{"type":"string"},"followUps":{"type":"array","items":
  {"type":"string"}}}}. Additive: the prose report is unaffected, and a worker
  that skips or botches the block reports that beside its prose rather than
  failing. Validated keywords are type/properties/required/items/enum/
  additionalProperties; anything else is ignored (it can under-constrain, never
  wrongly reject). Arbitrary result schemas are desktop-owned; the headless
  brain declines resultSchema rather than accepting it without validation.
- A fleet worker may finish with one fenced wks-escalation object instead when
  it is terminally blocked on authority or a decision. Its strict six-key
  payload is type, status, reason, requiredAuthorityOrDecision, changed, and
  nextAction. Resolve an in-scope authority or decision yourself; ask the user
  only before destructive, external, credential, cross-repo, or otherwise
  unauthorized action. A valid escalation is a terminal alternative to
  wks-result and gets its own wake card; a malformed block is ordinary prose
  and does not suppress missing or invalid result errors. This fixed
  host-authored contract is available on desktop and headless fleet dispatches
  even though headless declines arbitrary resultSchema.
- template + templateParams renders a library DISPATCH TEMPLATE (kind
  'dispatch') into the first message instead of you retyping the framing.
  Discover one cheaply with list_library({kind:"dispatch", id:"ship-task"}):
  each dispatch item carries a machine-readable "params" list — {name,
  required, default?} — so you learn what to fill without reading the body.
  A required param left unfilled REFUSES the spawn naming it; it is never
  silently defaulted. The spawn result then echoes the text that was actually
  sent back as "renderedMessage" (with renderedMessageTruncated:true if it was
  clipped), so verifying the render costs nothing — do not call
  get_conversation just to see what your own dispatch said.
- role, capability and decisionId carry a select_model answer onto the dispatch,
  alongside the provider, model and effort it named. role is what the work IS
  (implementer, reviewer, scout and the rest); capability is the model tier the
  answer chose, which you copy and never raise; decisionId joins this worker to
  that decision in the host's routing log. A dispatch that declares no role is
  routed by nobody: it gets no decision recorded, gives the directory ceiling no
  capability to judge, and makes no freshness claim, so a reviewer sent without
  one loses the guarantee that it never saw the implementation. The routing
  topic covers what to ask for and what binds whether you asked or not.
- YOUR ANSWER MAY COME BACK CLAMPED, and escalationScrubbed on the spawn result
  is where you see it: a list of the fields the host took away (capability,
  model, effort, toolScope, profileId are the ones you can cause). It is stamped
  by the host, which deletes any value you sent under that name first, so it
  reports what THIS spawn lost and nothing else. A clamped capability takes the
  model and effort you named with it and substitutes the permitted capability's
  own. Read it on every spawn rather than assuming you got what you asked for,
  tell the user what was narrowed, and do not retry the same request: only a
  person with a text editor can raise a ceiling.
- Drive it afterwards with send_message; watch it with get_conversation.
- To spawn on a federated peer machine, pass hub (a hub name seen on
  list_agents rows). The peer clamps remote spawns itself — permission bypass
  is refused there — and driving the new agent needs the same hub value.`),
	"routing": strings.TrimSpace(`
select_model is the ask-before-you-dispatch tool. You name the ROLE the work is
(scout, mechanical, implementer, reviewer, deep_reviewer, fixer, complex_fixer,
validator, diagnostician, judge) and the project directory as cwd, and the hub
resolves that role through its routing matrix and the live subscription capacity
into a concrete provider, model and effort. You never name a model yourself: the
matrix is the only place model names live, so a vendor rename is one file rather
than every dispatch site.
What comes back, and what to do with it:
- provider, model, effort: pass them to spawn_agent exactly as named.
- capability (the tier chosen) and baseCapability (what the role asks for before
  the routing mode moved it). Copy capability onto the spawn; there is no way to
  ask for one above what the answer gave.
- decisionId: pass it on the spawn so the worker and the decision are joined.
- mode (normal, conserve or spend_down), the capacity it judged, the ceiling it
  resolved under, and a reason list saying why. The reasons are what you quote to
  the user, not your own guess at them.
- capacity.pace, when the host has pacing on: the allowance judged against the
  CLOCK rather than as a level. A window is not just 60% used, it is 60% used
  with half its life left, and the ratio of the two is what says whether the
  fleet will run dry before the reset. pace.state is on_track, ahead,
  overspending or unknown; pace.ratio is consumed-over-expected (1.0 is exactly
  on the curve); pace.window names which window bound, and capacity.paceWindows
  shows what the others said. For a provider with two readable windows the WORSE
  one binds. Pace can turn a decision CONSERVE that health alone would have
  called normal, and it can block a spend_down; it never promotes anything and
  it never overrides a red or exhausted allowance. unknown (no window length
  reported, or nothing readable) changes nothing. Quote pace.because when you
  explain a conserve to the user — "we are 1.6x over the five-hour curve" is the
  sentence, and inventing your own number for it is not.
- effort, and effortStep when a routing mode moved it. A mode has two levers,
  not one: it can move the role to a different CAPABILITY (a different model),
  and it can step the effort one notch along that provider's own reasoning
  ladder — the SAME model, thinking less, or more. effortStep names the effort
  the matrix declares, the effort the answer runs at, and one sentence saying
  why. It is absent when nothing stepped. Pass effort to spawn_agent exactly as
  given; do not re-derive it from the model.
- eligible:false means the matrix found nothing spawnable for that role under
  that constraint: a provider held out of service, no profile pairing that
  capability with a provider you asked for, or a ceiling whose configured value
  cannot be read. model is empty and the reason list says which. Do not
  substitute a model of your own; report the reason it gave.
- previousProvider (the harness the previous worker ran on) is worth passing when
  you route a review, so the answer can land on a different model family. It is a
  PREFERENCE: the router tries the capability's other-provider candidates first
  and takes the best one it can actually use, then reports independentFamily
  honestly when none of them could be. fresh is the rule with teeth.
- fellOverFrom, when present, names the pairing the answer did NOT take. Two
  different mechanisms can move work across providers and they are easy to
  confuse: mode_shifts moves a ROLE to a DIFFERENT CAPABILITY (a conserving
  scout drops from balanced to cheap, and whichever provider that lands on is a
  consequence), while alternatives keep the SAME capability and change only
  which PROVIDER serves it, when the primary's own allowance is red, its
  provider is conserving, or its row is switched off. capability plus
  baseCapability tell you the first happened; fellOverFrom tells you the second
  did. A fallover can also be triggered by LIVE AVAILABILITY: when the host has
  just asked a provider's CLI what it can launch and the answer was nothing, the
  work goes to the next candidate and the reason names the provider that
  could not be started. A provider the host could not ASK about is unknown and
  is used as normal — "we could not check" is never read as "it is down".
ROUTING IS NOT AUTOMATIC, AND ASKING IS YOUR JOB. select_model is a read-only
question and nothing asks it for you: the host does not consult routing when it
spawns, so a dispatch that never asked carries no decision and appears nowhere in
the decision log. Believing your spawn was routed when you never asked is the way
to misread its result.
Two rules DO bind on every spawn the host receives, asked or not, because they
live in the spawn sanitizer rather than in routing:
- THE CEILING is per directory (longest matching ancestor wins, default
  otherwise) and caps two axes: the capability, and the tool tier a worker there
  may hold. A spawn above it is clamped rather than refused. The capability drops
  and takes the model and effort you named with it, replaced by the permitted
  capability's own; the tool tier is lowered the same way; escalationScrubbed on
  the result names what went. select_model applies the SAME ceiling before it
  answers, so a capped answer arrives already capped, with the ceiling and the
  reason on it, instead of being taken away afterwards.
- FRESHNESS is a refusal, not a downgrade. Every shipped profile marks its review
  capabilities fresh, and a spawn declaring a role or capability the active
  profile marks fresh may not also carry a resumeSessionId. The host refuses the
  call and names the session it would have inherited, because dropping the field
  would start a new session the caller went on believing was a continuation.
  spawn_agent carries no resumeSessionId field at all, so your own dispatches
  cannot trip it; the rule binds the bus spawn paths that do carry one. What a
  fresh reviewer gets instead is yours to compose: the task, the acceptance
  criteria, the diff, the files and the test results, and never the implementer's
  reasoning.
docs/limit-aware-routing.md in the workspacer repo is the full reference: the
matrix file, the profiles, the modes and the thresholds, and how to raise a
ceiling. Nothing on this wire edits any of it.`),
	"drive": strings.TrimSpace(`
send_message queues a prompt for an agent (delivered when it can accept input).
approve resolves a pending permission prompt (yes/no/always — "always" persists
a standing allow, so use it deliberately). answer resolves an AskUserQuestion
picker. signal sends SIGINT (interrupt) / SIGTERM (stop). set_approval_gate
parks every tool call for approval — useful before letting an agent continue
unattended. The gate is a workspacer-side hold, SEPARATE from the session's
Claude permission mode: gate off does not stop the session's own permission
prompts (only its permission mode governs those; the gate response reports the
current mode). terminal_input types raw bytes into a PTY session (shells from
create_terminal); prefer send_message for agents. For a session on a federated
peer (its list_agents row has a "hub" field), pass that hub value on
send_message / approve / answer / signal / set_approval_gate.
adopt_workers is for SUCCESSION, and you need it once, on your first turn as a
replacement manager. Fleet wakes are parent-keyed: a worker's finished report
and its report_progress notes go to the session that dispatched it, so when a
manager is replaced its dispatches are talking to a session that is gone.
adopt_workers({fromSessionId: <the manager you replaced>, toSessionId: <your
own session id>}) re-points them at you — after it, every one of those workers
wakes YOU when it finishes, with no polling and no reconciliation by hand. The
predecessor's id is on the first line of its handoff file. If it CRASHED and
wrote no handoff, call list_orphans: it returns every DEAD parent that still has
live children — the label it ran under, its directory, when it died, whether it
was confirmed to be a manager (the host keeps a record of a manager's death that
outlives its session row), and the workers still pointing at it. The confirmed
manager whose label and directory match what you were told to take over is your
fromSessionId. It reports candidates and never picks one, on purpose: with two
of them, adopting the wrong group re-points ANOTHER manager's workers onto you
and nothing says so — read a worker of each group first (its cwd, and
get_snapshot for what it was dispatched to do). A candidate marked
confirmedManager:false is only a dangling parent id, which could equally be a
worker that spawned agents of its own: a lead, not an answer. It is refused if you
are not a live manager session (re-pointing workers at a parent no wake can
reach would silence them, which is worse than leaving them), and "0 moved" is a
real answer: the predecessor had nothing left in flight.`),
	"files": strings.TrimSpace(`
Host-filesystem access (the machine workspacer runs on): list_dir / list_entries
to explore, read_file / write_file for file IO, search_project for ripgrep
across a project. Use these to inspect or brief work, not to do the coding
yourself — spawn an agent in the directory instead.`),
	"jobs": strings.TrimSpace(`
Jobs are recurring or one-off tasks the HUB runs unattended: spawn an agent
with a prompt, run a shell command, or call a capability, on an interval, at a
daily time, once, or manually. They keep firing with the app closed.
You can read them (list_jobs, job_history), run or delete an existing one, and
PROPOSE new ones. You cannot arm one. A proposal is saved disabled with your
name on it and never runs until the user approves it in Settings -> Jobs, so
tell the user it is waiting for review; do not report it as scheduled.
propose_job forces enabled:false and mints a fresh id whatever you send, so do
not bother setting either and do not tell the user that a value you sent for
them took effect.

THE SPEC. A job is {"name","enabled","trigger","action"}; leave "id" out or
blank to create. The hub validates on save and refuses with a message naming
the problem, so the constraints below are hard, not style.
- name: required, and blank after trimming counts as missing.
- trigger, exactly one kind, and there are only these four. There is no event
  trigger and no bus-topic trigger; do not invent one.
    {"kind":"interval","everyMinutes":60}   everyMinutes >= 1
    {"kind":"daily","at":"09:00","days":[1,2,3,4,5]}
        at is 24-hour HH:MM in the HUB's local time. days are 0=Sunday through
        6=Saturday, and omitting days means every day.
    {"kind":"once","once":"2026-09-01T09:00:00Z"}   RFC3339
    {"kind":"manual"}                       fires only when someone runs it
- action, exactly one kind:
    {"kind":"spawn","spawn":{"cwd","prompt","provider","model","effort",
                             "permissionMode","context"}}
        cwd and prompt are both REQUIRED and both refused when blank. cwd is
        an absolute path. provider is claude | codex | copilot | opencode | pi; blank
        model or effort means the provider's default. permissionMode is
        clamped for jobs whatever you ask for: an unattended agent gets no
        permission bypass, no pre-approved MCP servers and no account-profile
        config dir.
    {"kind":"shell","shell":{"command","cwd"}}
        command required; runs through /bin/sh -c (cmd /C on Windows). cwd
        optional.
    {"kind":"call","call":{"method","params"}}
        method required. It may NOT start with jobs. (that recurses into this
        surface) or hub: (that would run the call on another machine; job
        state is host-local and deliberately does not federate).
Runs are capped at 15 minutes. A fire that lands while the previous run is
still going is skipped, never queued. A once job disables itself when it
fires. A failed run raises a notification carrying the output tail.

CONTEXT STEPS are the part worth reaching for. A spawn action may run up to
FOUR steps first: code whose output is fed to the agent, and whose guards can
cancel the run so no model is woken at all.
  "context":[{"kind":"shell","shell":{"command":"go test ./...","cwd":""},
              "skipIfEmpty":true,"skipUnlessMatch":"FAIL","ignoreExitCode":true}]
A step is kind shell or call, and a call step obeys the same jobs. / hub: rule
as a call action. Output is substituted into the prompt at {{output}}, or at
{{output.1}}, {{output.2}} and so on when several steps run; a prompt naming
neither gets the outputs appended as fenced blocks rather than dropped. Each
step's output is capped at 12000 characters and elided in the MIDDLE, so both
what ran and how it ended survive. The guards:
  skipIfEmpty      nothing came back, so skip the run. For a call step, {},
                   [], null and "" all count as nothing.
  skipUnlessMatch  RE2, compiled when the job is SAVED, so a bad pattern is
                   refused then rather than silently never matching at 3am.
  ignoreExitCode   a nonzero exit code is data, not failure (grep finding
                   nothing, a test runner reporting failures). Only an exit
                   CODE is forgiven: a timeout or an unstartable command still
                   fails the run.
A guarded run records as skipped and, unlike a failure, stays silent. Prefer a
guarded spawn over an unguarded one: a job that wakes a model nightly to
answer "nothing to do" is waste the user pays for.

IS THE MACHINE IDLE? The hub answers fleet.quiescence, a read-only signal for
whether this machine's fleet is genuinely at rest, and the CLI over it is
"workspacer fleet quiescence" (exit 0 at rest, 1 not at rest, 2 could not ask;
2 is not a no). It refuses when unsure, so a session that is spawning, a live
background shell, an open terminal, a pending approval, a job due soon and an
unreachable federated peer all count as busy. It is what a shell job should
ask before doing anything that assumes nothing is happening, powering the
machine down being the obvious one:
  trigger {"kind":"interval","everyMinutes":5}
  action  {"kind":"shell","shell":{"command":
             "workspacer fleet quiescence --quiet && /path/to/their/script.sh"}}
A shell action does not count as a job due soon, because a shell action is how
this check gets run and a poller that counted itself would block forever.
Do not write the shutdown half for the user. work{spacer} powers nothing down
itself, the script is theirs, and Settings -> Jobs ships this as a template
that arrives switched off with the script path left blank. Send them there
rather than proposing a job that runs a script nobody has written.`),
	"lifecycle": strings.TrimSpace(`
Stopping a worker is TWO steps, and both are verbs now:
1. signal({sessionId, signal:"SIGTERM"}) stops the process. (SIGINT just
   interrupts the current turn.)
2. close_session({sessionId}) DISMISSES the row: it leaves list_agents and the
   fleet stops counting it. Before this existed, the only confirmation a worker
   had really died was sending it another signal and reading a 404.
respawn_with({sessionId, amendment}) is the OTHER half of that move: it clones
the stopped worker's ORIGINAL task and its cwd/model/provider/effort/parent,
appends your correction under a heading that supersedes anything above it, and
starts a fresh agent with both — so you state the DIAGNOSIS, not the whole task
again. Override model/effort/label/cwd/toolScope as needed; pass worktree:true
to start clean instead of continuing in the original's worktree. It refuses
without an amendment (a clone with no correction just repeats itself), and the
successor's permission mode is re-judged by the same grant check a fresh
spawn_agent gets — a bypassed original does not make a bypassed clone.

close_session refuses while the session is still working — hiding a running
agent from list_agents while it keeps spending is worse than a stale row — and
is idempotent, so closing an already-forgotten session succeeds. It stops the
daemon side too for a session that had not already ended, so "dismissed" is not
a lie. The user's desktop PANE is theirs to close; this is the fleet's view.`),
	"watch": strings.TrimSpace(`
notify_when({sessionId, contextUsedPct|tokens|usd|idleSeconds, notifySessionId?}) is how you
keep an eye on a running worker WITHOUT polling. Never loop on list_agents or
get_conversation to "keep an eye on" something — that is a hang, and it locks
the user out. Arm a watch and STOP.
- Prefer contextUsedPct (finite range (0,100]) for health: it is ACTIVE context
  occupancy divided only by a runtime-confirmed effective window. OpenCode and
  Pi cannot emit that window and are refused at arm time. It is single-purpose
  and cannot be combined with another threshold. Missing,
  stale, provisional, reset, or inconsistent telemetry leaves the watch armed
  until trustworthy telemetry arrives; an already-high sample fires on the
  next sweep. A provider/session/model reset invalidates it and asks you to
  re-arm. The wake includes percentage, numerator, denominator, provider,
  observation time, and telemetry epoch. Claude correlates the pair on result
  frames; during a long turn the prior sample may age stale and the watch waits
  rather than trusting an in-progress numerator.
- tokens is legacy cache-inclusive CUMULATIVE throughput (input + output,
  including cache reads where reported): useful for cadence/scope checkpoints,
  not active-context health. Compaction does not reset it. usd is cumulative
  session cost (provider-authoritative where supplied, otherwise Workspacer's
  estimate; absent cost telemetry cannot cross it);
  idleSeconds catches a worker that stopped without finishing — it measures
  SILENCE, not the reported state, so it also catches one that is wedged and
  still claiming to stream. The wake names which of the two it was.
- Crossing delivers a [fleet] wake naming exactly what crossed.
- ONE-SHOT: the watch is discarded when it fires. Arm another if you still want
  to watch — that is a decision you should make deliberately, not a loop.
- The wake goes to the target's PARENT by default (you, for a worker you
  dispatched); notifySessionId overrides it.
- Watches live in memory: a workspacer restart clears them; they are not
  persisted or recovered.`),
	"brief": strings.TrimSpace(`
brief_append({project, section, line}) adds ONE line to a project's
.workspacer/brief.md. Use it instead of read_file + write_file: it is
inspect-then-edit under a lock, so it cannot clobber a line a worker or the user
wrote while you were composing yours, and it is strictly additive — it never
rewrites, reorders or reformats anything already in the file.
- project is the project DIRECTORY (your own cwd for your fleet brief); the
  .workspacer/brief.md path under it is composed for you.
- section is Now | Direction | Recently | User. A typo is refused, not guessed.
- Recently PREPENDS (a dated log, newest first); the rest append.
- The brief is created, with all four sections, if the project has none.
- A line over 4000 characters is REFUSED and nothing is written, rather than cut
  at the limit. Split it into separate entries and append each one.
- The result carries entriesInSection, bytesInSection and bytesInBrief: the
  state AFTER your write. That is how you notice a brief going over budget
  without reading it. Past roughly 20 entries in a section, trim it.
- LOGGING A FINISHED WORKER: pass sessionId and the worker's parsed wks-result
  object, and write only your ONE SENTENCE of significance in 'line'. The host
  adds the date, renders the facts (commit, files, checks, caveats, follow-ups)
  compactly, and appends a VALIDATED session:<id>. You stop retyping mechanical
  detail, and you stop mistyping the reference — a malformed id is refused with
  nothing written rather than left in the brief as a dead link. The sentence is
  required: a result object alone is never a brief line, because the judgement
  is the half only you can write.
brief_check({project}) reports which ## Now lines have outlived their dispatch:
references to sessions this host no longer knows about (a FINISHED worker counts
as gone — that is exactly the case that strands a line), malformed references,
and dispatch-shaped lines that name no session at all. It is READ-ONLY and never
prunes anything: the user's brief edits are authoritative, so it hands you the
list and you decide. Worth a call when you inherit a fleet or run /checkpoint.
brief_archive({project, section, keep|count}) is the other half, and the way to
trim: it moves the OLDEST entries of one section out to
.workspacer/brief.archive.md in a single call, byte for byte, under the same
lock. Give keep (leave this many newest, archive the rest; idempotent) or count
(archive this many oldest), never both. Recently is newest-first, so its oldest
entries are its last. It returns how many moved plus the same three counts.
Between the two, do not edit a brief with read_file + write_file.`),
	"projects": strings.TrimSpace(`
project_status({}) returns the git state of EVERY configured project in one
call: branch, upstream, unpushed (commits ahead), behind, and whether the tree
is dirty. Use it for a standup instead of shelling out per repo. Pass dirs to
limit it. A directory that is not a repo comes back as a row carrying an error,
so one bad path never costs you the other rows. "unpushed" is ABSENT (not 0)
when a branch has no upstream — that is "nowhere to push", not "nothing to
push".`),
	"config": strings.TrimSpace(`
get_config returns the full workspacer config; save_config deep-merges a
partial patch (pass ONLY the keys you change). reload_config re-reads disk.`),
	"profiles": strings.TrimSpace(`
Claude profiles are named CLAUDE_CONFIG_DIR + extra-args presets applied at
spawn (spawn_agent's profileId). list/add/update/remove.`),
	"sessions": strings.TrimSpace(`
Saved workspace sessions are pane/agent ARRANGEMENTS (the session picker), not
live agents — list_agents is the live fleet. list/load/save/delete by filename.`),
	"layouts": strings.TrimSpace(`
Layout templates are saved pane-geometry presets. list/save/delete.`),
	"library": strings.TrimSpace(`
The library holds reusable prompts, skills, agent definitions and Fleet Manager
DISPATCH TEMPLATES, global or per-project (pass cwd). list/save/remove.
list_library takes optional kind and id filters — use them: an unfiltered
listing returns every item's full body, while list_library({kind:"dispatch"})
is the cheap way to see the templates you can spawn with. Every kind
'dispatch' row carries "params", the placeholders parsed out of its body
({name, required, default?}), which is what spawn_agent's templateParams
expects.`),
	"analytics": strings.TrimSpace(`
analytics_summary aggregates usage/cost across sessions; analytics_recent
returns the latest finished sessions with per-session usage.`),
	"report": strings.TrimSpace(`
report_progress({note, needsDecision?}) is how you talk to whoever dispatched
you, WITHOUT finishing. You cannot address it: it goes to your manager, derived
from your own credential, and there is no session id in the call.
- ONE line, about your own run: a phase landed, the approach you were given is
  wrong, you are burning context far faster than the task warrants. Say what
  would change your manager's next decision.
- It does NOT end your turn or your task, and it is not your report: your final
  message reaches your manager in full when you actually finish.
- needsDecision:true marks you BLOCKED on an answer. The channel is one-way —
  your manager may reply with a message, or may not — so keep working if you
  can, and do not wait on it.
- One per minute, 20 per session, no repeats. Every bound refuses out loud, so
  a rejected note was NOT delivered: shorten it, wait, or fold it into the next
  one. Nothing is ever silently truncated.
- If you have no manager (nobody dispatched you), it refuses — tell the user in
  your reply instead.`),
	"notify": strings.TrimSpace(`
notify shows a desktop notification on the workspacer machine — use it to
surface something that needs the user's attention.

GIVE IT A CLICK TARGET whenever there is somewhere to go. A notification that
describes the route in prose ("review it in Settings") makes the user navigate
by hand; one that carries a target takes them there.
- sessionId: select that agent. Highest priority — use it for anything about
  one agent.
- paneType (+ paneSection): open a pane. With paneType "settings", paneSection
  names the section, e.g. {"paneType":"settings","paneSection":"jobs"} for a
  job you just proposed. Without the section the Settings pane opens wherever
  the user last left it.
- url: opened in their browser. http(s) only; anything else is dropped.

Other fields: level (info/success/warn/error), key (a later notification with
the same key REPLACES this one instead of stacking), silent (record it in the
notification center without interrupting), inAppOnly (no OS notification).`),
	"ui": strings.TrimSpace(`
These steer the user's DESKTOP screen — use them deliberately.
- Right for guided tours ("show me around workspacer", first-time onboarding):
  narrate each step in chat, then focus_agent / open_pane / open_browser so the
  user sees what you describe. open_spawn_dialog shows how to start an agent
  without actually spawning one.
- open_browser opens the built-in browser pane on a URL — use it to walk the
  user through web docs (e.g. the workspacer guides) alongside your narration.
- open_guide opens the built-in Workspacer Guide pane — a chat with a dedicated
  tour agent. Hand off "how do I use workspacer" questions there instead of
  answering at length yourself.
- Otherwise PREFER notify: a notification lets the user click through when
  ready, while navigation yanks their screen mid-thought. Never re-focus
  repeatedly; once per step of a tour the user asked for.
- Fire-and-forget: "ok" means the command was sent, not that the UI acted. On a
  headless/remote-only setup nothing consumes these.`),
	"plugins": strings.TrimSpace(`
Plugin tools are contributed by installed workspacer plugins and forwarded to
the plugin's own sidecar. They appear only when your session was granted them
at spawn (pluginTools). Errors like "no provider" mean the plugin is not
running.`),
}

// addHelpTool registers the help tool on a tier's server, rendering from that
// tier's registry.
func addHelpTool(b *build) {
	tools := append([]toolInfo(nil), b.tools...)
	scope := string(b.scope)
	mcp.AddTool(b.s, &mcp.Tool{
		Name:        "help",
		Description: "Usage guide for the workspacer tools: call with no arguments for the grouped overview, or with a topic (group name) for detailed guidance. Call this before first using the other workspacer tools.",
	}, func(_ context.Context, _ *mcp.CallToolRequest, in helpIn) (*mcp.CallToolResult, any, error) {
		return &mcp.CallToolResult{
			Content: []mcp.Content{&mcp.TextContent{Text: renderHelp(scope, tools, in.Topic)}},
		}, nil, nil
	})
}

// renderHelp renders the overview (topic == "") or one group's detail.
// commonTools is the working set a fleet manager reaches for every session, in
// the order it needs them. It exists for ONE reason: an MCP client that DEFERS
// tool schemas (Claude Code does, when a server contributes many) makes the
// agent fetch each schema before first use — measured at ~6 separate round
// trips in a single manager session, before any work happened. The overview
// below renders these as a single batch query, so the tax is paid once.
//
// A hand-picked list rather than "every tool this tier holds": batching all ~50
// operator schemas would load ~10k tokens of context the tier system exists to
// avoid spending. It is filtered against the tier's actual registry, so a tier
// is never told to fetch a tool it does not hold, and an entry deleted from the
// registry simply drops out.
var commonTools = []string{
	"list_agents", "get_conversation", "spawn_agent", "send_message",
	"approve", "answer", "signal", "close_session", "respawn_with",
	"notify_when", "brief_append", "project_status", "notify",
}

// batchLoadHint renders the one-call schema fetch, or "" when the client
// clearly is not deferring (nothing to batch). Written as guidance rather than
// a command because the tool NAMES are the client's to spell: an MCP tool is
// usually exposed prefixed (mcp__workspacer__list_agents), and which prefix is
// the runtime's business, not ours.
func batchLoadHint(tools []toolInfo) string {
	held := map[string]bool{}
	for _, t := range tools {
		held[t.Name] = true
	}
	var picked []string
	for _, name := range commonTools {
		if held[name] {
			picked = append(picked, name)
		}
	}
	if len(picked) < 2 {
		return ""
	}
	return "\nIf your runtime DEFERS these tools' schemas (they are listed by name but not " +
		"callable until fetched), load the whole working set in ONE call rather than one per " +
		"tool — that round trip is otherwise paid several times a session, before any work " +
		"happens. The set, in the order you will want it:\n  " + strings.Join(picked, ", ") +
		"\nYour runtime prefixes MCP tool names (e.g. mcp__workspacer__list_agents); use the " +
		"spelling it lists, and fetch them together."
}

func renderHelp(scope string, tools []toolInfo, topic string) string {
	byGroup := map[string][]toolInfo{}
	var order []string
	for _, t := range tools {
		if _, seen := byGroup[t.Group]; !seen {
			order = append(order, t.Group)
		}
		byGroup[t.Group] = append(byGroup[t.Group], t)
	}

	topic = strings.ToLower(strings.TrimSpace(topic))
	if topic == "" {
		var sb strings.Builder
		fmt.Fprintf(&sb, "Workspacer tools — tier: %s. Call help with a topic for usage guidance.\n", scope)
		for _, g := range order {
			names := make([]string, 0, len(byGroup[g]))
			for _, t := range byGroup[g] {
				names = append(names, t.Name)
			}
			fmt.Fprintf(&sb, "- %s: %s\n", g, strings.Join(names, ", "))
		}
		if hint := batchLoadHint(tools); hint != "" {
			sb.WriteString(hint)
		}
		return strings.TrimRight(sb.String(), "\n")
	}

	group, ok := byGroup[topic]
	if !ok {
		known := append([]string(nil), order...)
		sort.Strings(known)
		return fmt.Sprintf("unknown topic %q — this tier's topics: %s", topic, strings.Join(known, ", "))
	}
	var sb strings.Builder
	fmt.Fprintf(&sb, "%s tools (tier: %s)\n", topic, scope)
	for _, t := range group {
		fmt.Fprintf(&sb, "- %s: %s\n", t.Name, t.Desc)
	}
	if g := groupGuidance[topic]; g != "" {
		fmt.Fprintf(&sb, "\n%s", g)
	}
	return sb.String()
}
