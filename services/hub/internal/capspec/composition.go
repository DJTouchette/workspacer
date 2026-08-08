package capspec

import "sort"

// COMPOSITION. Every guard in this package asks whether ONE call can escape.
// None of them asks whether TWO calls can escape together, and that omission has
// produced a critical in three consecutive hardening rounds. The three shared a
// shape, and generalising it gives two:
//
//	WRITE-THEN-INTERPRET  capability X writes bytes to a location capability Y
//	                      later reads as CONFIG, CODE, ARGV or POLICY rather than
//	                      as data. X is confined, Y is confined, and the boundary
//	                      crossed is "data becomes instruction".
//
//	WIDEN-THEN-USE        capability X changes state — a grant, a root set, a
//	                      permission mode, an approval gate, a session — that
//	                      capability Y's guard CONSULTS. X is allowed to change
//	                      it and Y is allowed to trust it, so neither is wrong.
//
// A pair is invisible to every per-call check by construction: both halves are
// correct. So the pairs are written down HERE, next to the per-method
// classification, and held to two machine-checkable claims —
//
//  1. a pair marked ClosedBy names the mechanism that makes it no stronger than
//     its halves, and that mechanism has a test of its own;
//  2. a pair marked AcceptedIn names the credential tiers that deliberately hold
//     both halves, and NO OTHER TIER may hold both.
//
// (2) is the part that keeps working after this round. `agents.sendMessage`'s
// own excuse says "the agent's own tool approvals are the gate"; the triage tier
// holds `claude.approve`, the RESOLVER of exactly those approvals. That is a real
// product decision — the phone's whole job is replying to an agent and answering
// its prompts — but it means the excuse names a bound that tier does not have,
// and until it was written here nothing would have noticed a THIRD half joining
// the same tier.

// CompositionShape is the closed vocabulary of ways two correct calls compose
// into something neither is.
type CompositionShape string

const (
	// ShapeWriteThenInterpret — X writes bytes, Y executes/obeys them.
	ShapeWriteThenInterpret CompositionShape = "write-then-interpret"
	// ShapeWidenThenUse — X changes state Y's guard consults.
	ShapeWidenThenUse CompositionShape = "widen-then-use"
)

// Composition is one recorded pair.
//
// A and B name either a capability method (which must be [Classified]) or an
// event topic pattern (which must be in the topic-guard table). The event side
// is a first-class half because one of these pairs crosses precisely the seam
// between the capability plane and the event plane.
type Composition struct {
	// Name is the one-line statement of what the pair does.
	Name string
	// Shape is which of the two generalised forms this is.
	Shape CompositionShape
	// A is the first half — the writer, or the widener.
	A string
	// B is the second half — the interpreter, or the consulter.
	B string
	// Crossing is the boundary that no per-call guard could see, in one
	// sentence: what each half is individually allowed to do, and what the two
	// of them are together.
	Crossing string
	// ClosedBy names the mechanism that makes the pair no stronger than its
	// halves. Mutually exclusive with AcceptedIn.
	ClosedBy string
	// AcceptedIn names the credential tiers that deliberately hold BOTH halves.
	// Mutually exclusive with ClosedBy. "operator" is never listed: it is the
	// full-authority tier and holds every pair by definition.
	AcceptedIn []string
}

// compositions is the record. Adding a pair is cheap; the value is that a
// FUTURE tier change, or a new capability that completes one of these shapes,
// has something to fail against.
var compositions = []Composition{
	{
		Name:     "fs.write plants a provider's hooks/permissions file; the next spawn in that cwd runs it unprompted",
		Shape:    ShapeWriteThenInterpret,
		A:        "fs.write",
		B:        "agents.spawn",
		Crossing: "fs.write is confined to the live agent cwds; agents.spawn is unconfined BY DECISION because starting a process is a separate authorization question. Every guard read `<cwd>/.claude/settings.json` as ordinary project DATA — inside a root, not a credential basename, no `.git` component — while Claude Code reads it as POLICY AND ARGV: a SessionStart hook runs as the desktop user before any model call, with no approval prompt and no permission mode. With no cwd at all the spawn normalizes to $HOME, which puts ~/.claude/settings.json in the same reach and makes the hook fire for every claude session on the host.",
		ClosedBy: "the agent-interpreted-config arm of the secret gate — pathIsAgentInterpretedConfig in cmd/brain/fsguard.go and internal/bus/policy.go, isAgentInterpretedConfigPath in pathConfinement.ts — pinned across all three copies by the `secrets` cases of contracts/path-containment-cases.json",
	},
	{
		Name:     "fs.write plants <cwd>/.opencode/plugin/*.js; providers.listModels executes it",
		Shape:    ShapeWriteThenInterpret,
		A:        "fs.write",
		B:        "providers.listModels",
		Crossing: "capspec's own excuse for providers.listModels' cwd said it 'picks which project's provider config to READ; the provider resolves the file itself'. opencode does not read that directory, it RUNS every .opencode/plugin/*.js in it at startup, before printing a model list, with no manifest and no other file required — and the consent list labelled the capability 'List available models'. A cwd that selects data needs no confinement; a cwd that selects an interpreter's plugin directory needs the confinement git.* got.",
		ClosedBy: "both halves: the plugin directory is refused by the same gate (pathIsAgentInterpretedConfig / isAgentInterpretedConfigPath), and the cwd moved from unscopedByDecision into PathParam (browse roots) with both providers asserting it — pinned by the `providers.listModels` row of the corpus `methods` block",
	},
	{
		Name:     "fs.write plants a ripgrep .ignore; search.project then returns the files the secret gate exists to refuse",
		Shape:    ShapeWriteThenInterpret,
		A:        "fs.write",
		B:        "search.project",
		Crossing: "search.project guards its CWD and nothing else, delegating per-file exclusion to ripgrep's hidden/ignore walker — whose policy is a file INSIDE the searched directory. `<root>/.ignore` holding `!*` is an ordinary dotfile to every guard here, so bytes written as DATA by one confined call became the READ POLICY of the next, and matching lines came back out of .git/config and .settings.json.",
		ClosedBy: "the read-set invariant: a capability that returns file CONTENT may not return bytes fs.read would refuse. resultPathIsSecret in cmd/brain/search.go and isSecretResultPath in pathConfinement.ts, applied per result path",
	},
	{
		Name:     "layout.set writes the shared document; the desktop's next launch respawns it through the LOCAL spawn door",
		Shape:    ShapeWidenThenUse,
		A:        "layout.set",
		B:        "agents.spawn",
		Crossing: "the hub stores the document verbatim because it 'does not interpret' it, and the desktop adopts it on hydration and respawns every stopped agent in it through window.electronAPI.spawnClaude — the LOCAL IPC door, which scrubs nothing. The bus's own agents.spawn refuses skipPermissions, an escalating permissionMode, a bypassing profile and caller-supplied mcpItemIds; all four arrived at the spawn anyway, from a caller that may not spawn at all.",
		ClosedBy: "layout.scrubAdoptedSpawnFields, applied to every NON-TRUSTED layout.set (the hub registers it through RegisterLocalIdent so the writer's identity is known), stripping exactly the four fields agents.spawn strips",
	},
	{
		Name:     "a plugin pane's ${agentCwd} binding comes out of the shared layout document, so that document sets the plugin sandbox's own boundary",
		Shape:    ShapeWidenThenUse,
		A:        "layout.set",
		B:        "fs.read",
		Crossing: "the bus's per-plugin path confinement is the ONE guard that is per-caller rather than per-host, and its root set is bound from the pane the host is opening — a pane read out of the shared layout document. An agent with cwd '/' produced a pane token whose fsRoots were ['/'], and a volume root contains everything below it, so the plugin's fs.* capabilities then admitted every path on the machine. Neither call is wrong: layout.set writes an opaque document and PaneToken faithfully binds what the trusted host hands it.",
		ClosedBy: "expandScope refuses a bare-token binding that RESOLVES to a volume root (internal/plugin/manager.go isVolumeRoot), the narrowing the subpath branch already had",
	},
	{
		Name:     "replay.open cuts a worktree under a grant; replay.read keeps serving it after the grant is revoked",
		Shape:    ShapeWidenThenUse,
		A:        "replay.open",
		B:        "replay.read",
		Crossing: "replay.open is confined to workspaceRoots because it cuts a worktree from the repo at cwd; replay.read is excused on the grounds that containment is 'structural'. It was — but keyed by a CALLER-CHOSEN sessionId in a process-global map whose only eviction is an explicit close. Once the session stopped, fs.read on the repo was refused and a fresh replay.open on it was refused, and replay.read went on returning its bytes to anyone who knew the id — which agents.list and sessions.snapshots hand out while classified inert.",
		ClosedBy: "guardReplaySession in hubCapabilities.ts re-runs replay.open's own containment on the entry's recorded origin cwd before every read/diff/seek",
	},
	{
		Name:     "the capability plane refuses sessions.attachTerminal to a view token; the event plane delivered its entire output",
		Shape:    ShapeWidenThenUse,
		A:        "sessions.attachTerminal",
		B:        "pty.bytes.*",
		Crossing: "two authorization planes answering the same question differently. mayCall denies the method to a scoped tier; mayConsume read `cn.trusted || cn.scopeMethods != nil || …`, whose middle clause waved every topic through for any scoped user token. terminals.* is in neither scoped tier at all, so the event plane was the only door onto a terminal's screen — raw PTY bytes with the ring-buffer replay attaching deliberately restarts.",
		ClosedBy: "the event-topic registry (eventtopics.go) consulted by mayConsume via EventTopicSpec, whose DEFAULT IS CLOSED for a scoped user token and which now also filters the plugin arm — plus the enqueue-time admission filter, so a refused stream no longer even leaves a drop record to escape as pty.desync",
	},
	{
		Name:     "sessions.save writes the boot-restore document; the desktop's next launch respawns it through the LOCAL spawn door",
		Shape:    ShapeWidenThenUse,
		A:        "sessions.save",
		B:        "agents.spawn",
		Crossing: "layout.set's recorded pair, reached through a DIFFERENT writer that was never scrubbed. sessions.save stamps `timestamp: now` into <configDir>/sessions/<slug>.yaml, which makes it sessions[0]; useSessionLifecycle loads it on boot, migrateSessionData passes the modern format through as-is, and reconcileAgents{respawnStopped:true} hands every card claudemon no longer holds to respawnFromRecord — which forwards profileId, permissionMode, skipPermissions and mcpItemIds to window.electronAPI.spawnClaude, the LOCAL IPC door that scrubs nothing. capspec excused the method as a PATH question (\"the filename is derived from the session name by the provider\u0027s slug\") and nothing in either provider looked at what the document CONTAINS.",
		ClosedBy: "scrubBootDocumentAgents, applied unconditionally on both providers (cmd/brain/bootdoc.go and main/lib/bootDocumentScrub.ts) because caller identity does not reach a bus provider — stripping exactly the four fields internal/layout\u0027s scrubAdoptedSpawnFields strips, with the three lists held equal by a test",
	},
	{
		Name:     "layouts.save writes the same agents array into the layout template the Layouts menu restores",
		Shape:    ShapeWidenThenUse,
		A:        "layouts.save",
		B:        "agents.spawn",
		Crossing: "the third copy of the boot-restore shape: <configDir>/layouts/<slug>.yaml holds \"the caller\u0027s whole agents array\" and is restored from the Layouts menu into the same loadAgentsFromSession -> reconcileAgents -> respawnFromRecord path as a saved session. One document shape, three writers, and the composition record named one of them — which is precisely how a closed chain stays reachable through a second door.",
		ClosedBy: "scrubBootDocumentAgents on both providers, the same call the sessions.save pair is closed by",
	},
	{
		Name:     "push.subscribe records an outbound network sink; agents.sendMessage pulls the trigger that makes the host use it",
		Shape:    ShapeWidenThenUse,
		A:        "push.subscribe",
		B:        "agents.sendMessage",
		Crossing: "push.subscribe stores a row; a DIFFERENT subsystem (push.Watch -> onSnapshot -> sendOne) consults that row to issue POST <endpoint> with a VAPID header from the HOST\u0027s network position — Tailscale-reachable, loopback-reachable, cloud-metadata-reachable — for a tier holding no fetch, no exec, no fs and no config capability. The trigger is the un-blocked -> blocked edge, and the same triage tier holds agents.sendMessage and claude.approve, so it can drive an agent into and out of that state on demand. capspec\u0027s excuse reasoned entirely about what the ENDPOINT learns (\"the payload is encrypted to the subscription\u0027s own keys\") and never about what the HOST is made to do.",
		ClosedBy: "validatePushEndpoint (internal/push/endpoint.go): https only, and no loopback, private, link-local (169.254.169.254) or unique-local host — the shape a browser PushManager actually produces",
	},
	{
		Name:       "agents.sendMessage injects the instruction and claude.approve resolves the prompt it raises",
		Shape:      ShapeWidenThenUse,
		A:          "agents.sendMessage",
		B:          "claude.approve",
		Crossing:   "agents.sendMessage's own excuse is that 'the agent's own tool approvals are the gate', and claude.approve is the RESOLVER of exactly those approvals — its own entry says so. A tier holding both can tell an agent that may run a shell to run one, and then approve it, without holding terminals.create, sessions.terminalInput, fs.write, git.* or agents.spawn. claude.gate is NOT a prerequisite: gate only ADDS parking, and the agent's own prompts already exist. `decision:\"always\"` persists a standing allow, so subsequent calls of that tool are not parked at all.",
		AcceptedIn: []string{"triage"},
	},
}

// Compositions returns the recorded pairs, ordered by name so a failing test
// prints the same thing twice.
func Compositions() []Composition {
	out := append([]Composition(nil), compositions...)
	sort.Slice(out, func(i, j int) bool { return out[i].Name < out[j].Name })
	return out
}

// CompositionHalves returns every method or topic named as a half of a recorded
// composition, so a tier guard can ask "does this credential hold both".
func CompositionHalves() []string {
	seen := map[string]bool{}
	for _, c := range compositions {
		seen[c.A] = true
		seen[c.B] = true
	}
	out := make([]string, 0, len(seen))
	for h := range seen {
		out = append(out, h)
	}
	sort.Strings(out)
	return out
}

// ── THE FORCING FUNCTION ────────────────────────────────────────────────────
//
// The record above is eight pairs somebody typed. The tier guard fires only when
// a tier holds both halves of a RECORDED pair, so it catches a TIER acquiring a
// known pair and NEVER a new capability forming an unrecorded one. That is the
// same defect shape this whole effort keeps fixing: a hand-maintained record
// that drifts from the thing it describes. It was confirmed by execution —
// injecting a new capability whose own excuse literally described a
// write-then-interpret half left every composition guard green.
//
// DERIVATION WAS INVESTIGATED AND REJECTED, on measurements rather than taste.
// A lens tried to derive the pairs from the 73-row capability inventory:
//
//   - the best rule reaching full recall on the reachable pairs emitted 525 of
//     5256 ordered pairs — 0.95% precision, 99.05% false positives;
//   - four of its five hits were carried by English filler ("arbitrary", "same",
//     "true", "nothing"), not by semantics: deleting one word from one prose cell
//     deletes the chain;
//   - ranking does not rescue it — a reviewer must read 587 of 1228 candidates
//     (48%) to see all five, and the top 100 contains none;
//   - the root cause is structural and was measured: only 6% of `executes` cells
//     name a joinable object at all, so there is nothing to join on. A
//     composition is a statement about a shared OBJECT and the inventory records
//     the VERB;
//   - and the fact that makes each recorded chain work — <cwd>/.claude/settings.json,
//     .opencode/plugin/*.js, ripgrep's .ignore — appears in NO row, because it is
//     a fact about a THIRD-PARTY interpreter that lives outside this repo.
//
// (There is weak real signal: shuffling the prose across methods drops recall
// from 5/5 to a mean of 0.84, p < 0.005. Topically adjacent is not composes.)
//
// So: not derivable, and therefore FORCED. compositionInert is the other half of
// the record — every capability that carries a caller value the host acts on
// must either be a half of a recorded pair or say, in writing, why it is not.
// "Considered and found inert" becomes checkable; silence stops being
// indistinguishable from it.

// compositionInert is the written record of "considered, and it cannot be half
// of a composition".
//
// A reason must answer the two shapes:
//
//	WRITE-THEN-INTERPRET  do the bytes this call writes end up somewhere another
//	                      capability, the host, or a third-party interpreter
//	                      reads as CONFIG, CODE, ARGV or POLICY?
//	WIDEN-THEN-USE        does this call change state — a grant, a root set, a
//	                      permission mode, an approval gate, a session — that
//	                      some other guard CONSULTS?
//
// "It is read-only" is a real answer. "It is safe" is not: every half of every
// pair above is safe.
var compositionInert = map[string]string{
	// ── reads that produce no durable state ────────────────────────────────
	"fs.read":                  "", // recorded half; listed for the guard's own completeness check
	"fs.readImage":             "returns decoded image bytes to the caller and writes nothing. Same confinement as fs.read, no interpreter downstream: nothing in the host re-reads an image as configuration",
	"fs.listEntries":           "returns names and types under a confined root. Composed with fs.write it is the shell-shaped pair the round-5 record already closes at the CONTAINMENT level (one predicate, 137 cases); it writes nothing itself and no guard consults its output",
	"fs.listDir":               "the same enumeration as fs.listEntries with a different result shape; identical reasoning",
	"fs.unwatch":               "removes a watcher this caller installed. It can only ever SHRINK what fs.changed carries, and no guard consults the watcher set",
	"git.status":               "runs `git status` in a confined cwd and returns text. Writes nothing; the porcelain output is not read as policy by anything",
	"git.log":                  "reads commit metadata out of a confined repo and returns it. Writes nothing, changes no state, and no guard in the system consults commit history when deciding anything",
	"git.numstat":              "reads per-file change counts for a commit range in a confined repo. Numbers to a UI: nothing is written, and nothing downstream reads the result as configuration or argv",
	"git.commitDiff":           "reads one commit's patch text out of a confined repo, under the same result-path secret gate git.diff has, so it cannot return bytes fs.read would refuse. Writes nothing",
	"git.commitNumstat":        "reads one commit's change counts in a confined repo — the per-commit twin of git.numstat, with the same absence of a writer and of a downstream interpreter",
	"claude.sessionsForDir":    "lists claudemon's known sessions for a directory. Read-only, and the ids it returns are already handed out by agents.list and sessions.snapshots",
	"claude.handoffBrief":      "renders a deterministic handoff brief into ~/.workspacer/handoffs/ and returns its path. The successor agent's composer is PRE-FILLED with 'read this file' rather than instructed by it, and the file is prose, not argv — the interpreter is a human reading a chat box. Its argv/profile fields come from the handoff builder, not the caller",
	"claude.handoffAgentBrief": "the per-agent variant of claude.handoffBrief; same builder, same output location, same reasoning",
	"replay.diff":              "reads a diff out of a worktree replay.open cut. Its containment is the recorded replay.open→replay.read pair's, re-run per call by guardReplaySession",
	"replay.seek":              "moves a cursor inside a replay session; same guard, no bytes leave the worktree that replay.read would not also return",
	"library.list":             "enumerates prompt/skill markdown under a confined cwd-derived directory. The items are inserted into a composer for a human to send, not executed; the directory itself is the confined thing",

	// ── writes whose bytes have no interpreter ─────────────────────────────
	"library.save":           "writes a .md prompt/skill into the confined library directory. The bytes become CHAT TEXT a human sends, never argv or config — and the one place a library item IS interpreted (a skill file) is generated by the host, not by this call",
	"library.remove":         "deletes one library item. Removal cannot introduce an interpreter, and nothing consults the item set as policy",
	"sessions.delete":        "deletes a saved session document. It can only remove a boot-restore document, never add one — the ADD direction is the recorded sessions.save→agents.spawn pair",
	"layouts.delete":         "deletes a saved layout template; same direction, same reasoning as sessions.delete",
	"sessions.load":          "reads a saved session document back. The document's dangerous direction is its WRITER (the recorded pair); reading it hands the caller bytes it could have written",
	"sessions.transcript":    "returns a session's transcript text. Read-only, and the transcript is rendered, never executed",
	"config.save":            "writes config.yaml, which IS re-read as argv by the host — and that is exactly why every such key (agents.binaries, claude.profiles, terminal.shell, terminal.shells, editor.terminalCommand, scripts, updates) is classified per-parameter in capspec.Params and STRIPPED from a bus write by dropHostTrusted, held equal to contracts/host-trusted-config-cases.json. The composition is real, it is closed per-key rather than per-pair, and the per-key record is the one that cannot drift silently",
	"claude.profiles.add":    "persists a profile whose configDir becomes CLAUDE_CONFIG_DIR and whose extraArgs become argv — the interpreter is real, and it is closed by scrubProfileBypass at write time on BOTH providers, with agents.spawn refusing a bypassing profileId from a bus caller as the second half",
	"claude.profiles.update": "the same fields through the same scrub as claude.profiles.add",
	"claude.profiles.remove": "removes a profile. Cannot introduce a configDir or extraArgs; a spawn naming a removed profile falls back to the default",
	"notifications.post":     "renders a title/body into an OS notification and an in-app card. Text to a human; no argv, no file, and nothing consults the notification set",
	"push.unsubscribe":       "drops a push subscription, proven by possession of the subscription's own auth secret. Removal only",
	"push.revoke":            "drops another credential's push subscriptions — a revocation, i.e. the direction that can only narrow",

	// ── state changes no other guard consults ──────────────────────────────
	"claude.setModel":          "switches the model of a running agent. No guard anywhere reads the model; the permission mode, which several do read, is a different method",
	"claude.setEffort":         "switches a running agent's reasoning effort. Like the model, effort is a parameter of generation that no guard anywhere consults — it is not the permission mode, which several do",
	"claude.signal":            "sends an interrupt/stop signal to a running agent. It can only ever STOP work; nothing consults it",
	"claude.gate":              "parks a tool call for human approval — it only ADDS a gate. Removing one is claude.approve, which is a recorded half",
	"claude.answer":            "answers an agent's question prompt. Text into a running agent, gated by the agent's own approvals exactly as agents.sendMessage is — and that pair (agents.sendMessage + claude.approve) is recorded and accepted for the triage tier, which is where this method's composition risk already lives",
	"claude.setPermissionMode": "changes the mode a running agent runs in, which IS state a later guard consults — and it is closed by the shared escalation allow-list (isPermissionEscalation / lib/permissionBypass.ts, one list, held equal by a test) refusing a bypassing mode from a bus caller, the same clamp agents.spawn applies",
	"sessions.terminalInput":   "types bytes into a session's PTY. Its OUTPUT side (sessions.attachTerminal / pty.bytes.*) is a recorded pair; the input side reaches a shell that is already running as the user, which is what terminals.create's own allow-list governs",
	"sessions.attachTerminal":  "", // recorded half
	"terminals.create":         "starts a shell, and its argv[0] is closed by the shell allow-list (lib/shellAllowlist.ts + cmd/brain/shellallow.go, one list held equal by a corpus). The config keys that could redirect that argv are stripped by dropHostTrusted — the two halves of that chain are classified where they live",
	"git.stage":                "stages paths inside a confined repo. The index is git's own state; no capability here reads it as policy, and the commit that consumes it is git.commit",
	"git.unstage":              "removes paths from a confined repo's index — the inverse of git.stage, so it can only ever shrink what a later git.commit records, and nothing reads the index as policy",
	"git.commit":               "records a commit in a confined repo. A commit message is not interpreted by anything in this system, and hooks in .git/hooks are refused by the secret gate that covers .git",
	"git.push":                 "publishes commits to the repo's configured remote. The remote URL and every exec-adjacent git config key are classified per-parameter (round 4's finding: 'the fix named one exec key; git config has a dozen'), which is where that composition is closed",
	"git.diff":                 "reads file contents out of the repo at a confined cwd, and its result-path secret gate is the recorded fs.write→search.project pair's closer applied to the same read-set invariant",
	"replay.open":              "", // recorded half
	"replay.read":              "", // recorded half
	"layout.set":               "", // recorded half
	"layouts.save":             "", // recorded half
	"sessions.save":            "", // recorded half
	"agents.spawn":             "", // recorded half
	"agents.sendMessage":       "", // recorded half
	"claude.approve":           "", // recorded half
	"fs.write":                 "", // recorded half
	"fs.watch":                 "installs a change watcher on a confined path. Its OUTPUT is the fs.changed topic, and that is where its composition lives: the event registry requires this capability to receive that topic, so a credential refused fs.watch is refused the change feed it produces. The call itself writes nothing and no guard consults the watcher set",
	"search.project":           "", // recorded half
	"providers.listModels":     "", // recorded half
	"push.subscribe":           "", // recorded half
}

// CompositionInertReason returns the written reason a capability was found
// unable to be half of a composition, and whether one is recorded. An empty
// reason with ok=true means the method IS a recorded half and needs no separate
// statement.
func CompositionInertReason(method string) (string, bool) {
	r, ok := compositionInert[method]
	return r, ok
}

// CompositionUnconsidered returns every capability that carries a caller value
// the host acts on and has neither been recorded as a composition half nor been
// written down as inert. The list is the forcing function's output: it must be
// empty.
func CompositionUnconsidered() []string {
	halves := map[string]bool{}
	for _, h := range CompositionHalves() {
		halves[h] = true
	}
	var out []string
	for _, m := range compositionActors() {
		if halves[m] {
			continue
		}
		if _, ok := compositionInert[m]; ok {
			continue
		}
		out = append(out, m)
	}
	sort.Strings(out)
	return out
}

// compositionActors is the population the forcing function covers: every method
// that carries a caller value the host acts on — PathParam (a path) plus
// unscopedByDecision (a value deliberately not confined). inertMethods are
// excluded by capspec's own definition ("carry NO caller value which becomes
// something the host acts on"), which is the same predicate a composition needs
// on at least one side.
func compositionActors() []string {
	seen := map[string]bool{}
	for m := range PathParam {
		seen[m] = true
	}
	for _, m := range UnscopedMethods() {
		seen[m] = true
	}
	out := make([]string, 0, len(seen))
	for m := range seen {
		out = append(out, m)
	}
	sort.Strings(out)
	return out
}
