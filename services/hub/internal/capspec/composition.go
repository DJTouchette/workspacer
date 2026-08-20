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
//     its halves, and that mechanism is PROVEN to act on one of the pair's own
//     halves (see Bearing);
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
//
// (1) was DECORATION until this round, and its failure mode is worth stating
// because it is the one this whole effort keeps re-learning. The only check on a
// ClosedBy sentence was "it cites a symbol that exists somewhere in the tree".
// So a real reason could be replaced with an unrelated live symbol and the suite
// stayed green — and worse, the tier guard in internal/authtoken SKIPS any pair
// that is ClosedBy, on the record's own definition that a closed pair is no
// stronger than its halves. A brand-new pair whose halves a scoped tier
// genuinely holds (sessions.snapshot + agents.sendMessage in `triage`) could
// therefore be silenced by fabricating an exemption: mark it closed, cite any
// live symbol, and the guard that exists to catch exactly that acquisition
// skipped it. An exemption anybody can mint is not an exemption.
//
// The fix is the same one that worked for unscopedByDecision's provider claims
// (TestUnscopedByDecisionProviderClaimsAreTrue): an excuse is held to its own
// word. A closure now carries a [Bearing] — a proof, in a closed vocabulary,
// that the named guard REACHES one of this pair's own halves.

// CompositionShape is the closed vocabulary of ways two correct calls compose
// into something neither is.
type CompositionShape string

const (
	// ShapeWriteThenInterpret — X writes bytes, Y executes/obeys them.
	ShapeWriteThenInterpret CompositionShape = "write-then-interpret"
	// ShapeWidenThenUse — X changes state Y's guard consults.
	ShapeWidenThenUse CompositionShape = "widen-then-use"
)

// ── PROVING THAT A GUARD BEARS ON A CAPABILITY ──────────────────────────────
//
// A Bearing is the machine-checkable half of the sentence "this pair is closed
// by X". Every kind below is verified against SOURCE, not against prose, by
// TestClosedCompositionsProveTheirGuardReachesTheirHalves.

// BearingKind is the closed vocabulary of proofs.
type BearingKind string

const (
	// BearsAtCallSite — the strongest form, and the one the provider-claim guard
	// already uses: the guard chain is ENTERED at a site that names the
	// capability itself (`guardGitCwd('git.status', …)`, or a handler whose body
	// carries the method name as a literal), and every link from that entry to
	// the named guard is a call in the previous link's own body.
	BearsAtCallSite BearingKind = "call-site"
	// BearsInTopicRegistry — for a pair whose halves straddle the capability and
	// event planes: capspec's own topic registry binds the topic half to the
	// method half. This one is checked in-process rather than textually, because
	// the registry IS the mechanism.
	BearsInTopicRegistry BearingKind = "topic-registry"
	// BearsOnGrantedRoots — the guard narrows the ROOT SET the bus confines every
	// path-scoped capability to, rather than sitting on one method's own path.
	// Deliberately the weakest kind, and its limit is stated rather than hidden:
	// it proves the guard governs the roots a path-scoped half is confined to, not
	// that it governs that half in particular. It is admissible only for a half
	// [IsPathScoped] actually returns true for, which is what keeps a fabricated
	// pair between two non-path capabilities from reaching for it.
	BearsOnGrantedRoots BearingKind = "granted-roots"
)

// Site is one link: a symbol and the repo-relative file that must contain it.
type Site struct {
	Symbol string
	File   []string
}

// Bearing is a proof that Symbol acts on the capability named by On.
type Bearing struct {
	Kind BearingKind
	// On is the capability (or event topic) the guard is claimed to reach. For a
	// composition it must be one of the pair's own two halves; for an inert
	// claim it must be the method the claim is about.
	On string
	// Symbol is the guard named in the prose. The prose must cite it, so the
	// sentence a human reads and the symbol a machine checks cannot drift.
	Symbol string
	// Entry is where the chain is entered.
	Entry Site
	// ByArg means Entry.File contains a call `Entry.Symbol("On", …)` — the guard
	// is invoked with the capability's own name, so it cannot be attributed to a
	// neighbouring registration. Otherwise Entry.Symbol must be a Go function
	// defined in Entry.File whose BODY carries "On" as a literal.
	ByArg bool
	// Chain is Entry.Symbol's definition followed by each call hop to Symbol.
	// Empty means Entry.Symbol is itself Symbol.
	Chain []Site
}

// Repo-relative files the proofs read. Named once so a move is one edit and a
// stale path is a failure rather than a skip.
var (
	desktopCapsFile   = []string{"apps", "desktop", "src", "main", "services", "hubCapabilities.ts"}
	brainHandlersFile = []string{"services", "hub", "cmd", "brain", "handlers.go"}
	brainFsguardFile  = []string{"services", "hub", "cmd", "brain", "fsguard.go"}
	brainSearchFile   = []string{"services", "hub", "cmd", "brain", "search.go"}
	brainProvFile     = []string{"services", "hub", "cmd", "brain", "providers.go"}
	hubLayoutFile     = []string{"services", "hub", "internal", "layout", "layout.go"}
	hubMainFile       = []string{"services", "hub", "cmd", "hub", "main.go"}
	hubPluginMgrFile  = []string{"services", "hub", "internal", "plugin", "manager.go"}
	hubPushFile       = []string{"services", "hub", "internal", "push", "push.go"}
	hubPushEndptFile  = []string{"services", "hub", "internal", "push", "endpoint.go"}
)

// argBearing is the common shape: a guard called with the capability's own name,
// and nothing between the call and the guard.
func argBearing(symbol, on string, file []string) Bearing {
	return Bearing{Kind: BearsAtCallSite, On: on, Symbol: symbol, Entry: Site{symbol, file}, ByArg: true}
}

// secretGateBearing is the fs secret gate reached from a capability's own
// assertPathAllowed call: assertPathAllowed → pathIsSecretCanonical →
// pathIsAgentInterpretedConfig. The last hop is the arm that refuses a provider
// CLI's hooks/permissions/plugin files, which is what closes the
// write-then-interpret pairs.
func secretGateBearing(on string, entryFile []string) Bearing {
	return Bearing{
		Kind: BearsAtCallSite, On: on, Symbol: "pathIsAgentInterpretedConfig",
		Entry: Site{"assertPathAllowed", entryFile}, ByArg: true,
		Chain: []Site{
			{"assertPathAllowed", brainFsguardFile},
			{"pathIsSecretCanonical", brainFsguardFile},
			{"pathIsAgentInterpretedConfig", brainFsguardFile},
		},
	}
}

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
	// Bearings prove the ClosedBy sentence. Required whenever ClosedBy is set,
	// and every Bearing must be On one of this pair's own halves: that is the
	// difference between "the symbol exists" and "the guard reaches this pair".
	Bearings []Bearing
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
		Name:     "jobs.upsert persists argv (a shell command or an agent spawn); the scheduler and jobs.run execute it later, unattended",
		Shape:    ShapeWriteThenInterpret,
		A:        "jobs.upsert",
		B:        "jobs.run",
		Crossing: "the job spec is BUILT to be interpreted: a shell action's `command` goes to /bin/sh -c on the hub's machine, a spawn action re-enters agents.spawn with a cwd and a prompt, and the trigger fires with nobody watching. Storage is the hub-owned 0600 jobs.json — deliberately NOT the library (agent-writable) or the layout (world-readable, broadcast) — so the file itself is out of reach; the bus surface is the remaining door.",
		ClosedBy: "identity, not paths — jobsTrusted: there is no subtree to confine a shell command to (the terminals.create argument), so every jobs.* handler in cmd/hub refuses non-trusted callers via jobsTrusted called with the method's own name — plugin tokens and view/triage tiers never reach the store. A spawn action's second hop re-enters agents.spawn as a bus caller and inherits its clamps (no bypass, no mcpItemIds, profile configDir scrubbed).",
		Bearings: []Bearing{
			argBearing("jobsTrusted", "jobs.upsert", hubMainFile),
			argBearing("jobsTrusted", "jobs.run", hubMainFile),
		},
	},
	{
		Name:     "fs.write plants a provider's hooks/permissions file; the next spawn in that cwd runs it unprompted",
		Shape:    ShapeWriteThenInterpret,
		A:        "fs.write",
		B:        "agents.spawn",
		Crossing: "fs.write is confined to the live agent cwds; agents.spawn is unconfined BY DECISION because starting a process is a separate authorization question. Every guard read `<cwd>/.claude/settings.json` as ordinary project DATA — inside a root, not a credential basename, no `.git` component — while Claude Code reads it as POLICY AND ARGV: a SessionStart hook runs as the desktop user before any model call, with no approval prompt and no permission mode. With no cwd at all the spawn normalizes to $HOME, which puts ~/.claude/settings.json in the same reach and makes the hook fire for every claude session on the host.",
		ClosedBy: "the agent-interpreted-config arm of the secret gate — pathIsAgentInterpretedConfig in cmd/brain/fsguard.go and internal/bus/policy.go, isAgentInterpretedConfigPath in pathConfinement.ts — pinned across all three copies by the `secrets` cases of contracts/path-containment-cases.json",
		Bearings: []Bearing{secretGateBearing("fs.write", brainHandlersFile)},
	},
	{
		Name:     "fs.write plants <cwd>/.opencode/plugin/*.js; providers.listModels executes it",
		Shape:    ShapeWriteThenInterpret,
		A:        "fs.write",
		B:        "providers.listModels",
		Crossing: "capspec's own excuse for providers.listModels' cwd said it 'picks which project's provider config to READ; the provider resolves the file itself'. opencode does not read that directory, it RUNS every .opencode/plugin/*.js in it at startup, before printing a model list, with no manifest and no other file required — and the consent list labelled the capability 'List available models'. A cwd that selects data needs no confinement; a cwd that selects an interpreter's plugin directory needs the confinement git.* got.",
		ClosedBy: "both halves: the plugin directory is refused by the same gate (pathIsAgentInterpretedConfig / isAgentInterpretedConfigPath), and the cwd moved from unscopedByDecision into PathParam (browse roots) with both providers asserting it — pinned by the `providers.listModels` row of the corpus `methods` block",
		// Both halves, because this pair is closed at both ends: the writer
		// cannot plant the file and the reader cannot be aimed at one.
		Bearings: []Bearing{
			secretGateBearing("fs.write", brainHandlersFile),
			secretGateBearing("providers.listModels", brainProvFile),
		},
	},
	{
		Name:     "fs.write plants a ripgrep .ignore; search.project then returns the files the secret gate exists to refuse",
		Shape:    ShapeWriteThenInterpret,
		A:        "fs.write",
		B:        "search.project",
		Crossing: "search.project guards its CWD and nothing else, delegating per-file exclusion to ripgrep's hidden/ignore walker — whose policy is a file INSIDE the searched directory. `<root>/.ignore` holding `!*` is an ordinary dotfile to every guard here, so bytes written as DATA by one confined call became the READ POLICY of the next, and matching lines came back out of .git/config and .settings.json.",
		ClosedBy: "the read-set invariant: a capability that returns file CONTENT may not return bytes fs.read would refuse. resultPathIsSecret in cmd/brain/search.go and isSecretResultPath in pathConfinement.ts, applied per result path",
		Bearings: []Bearing{{
			Kind: BearsAtCallSite, On: "search.project", Symbol: "resultPathIsSecret",
			// The handler names the method in its own body; from there the hops
			// are real calls: searchProject → the collector's addLine → the
			// per-result gate.
			Entry: Site{"searchProject", brainHandlersFile},
			Chain: []Site{
				{"searchProject", brainHandlersFile},
				{"searchProject", brainSearchFile},
				{"addLine", brainSearchFile},
				{"resultPathIsSecret", brainSearchFile},
			},
		}},
	},
	{
		Name:     "layout.set writes the shared document; the desktop's next launch respawns it through the LOCAL spawn door",
		Shape:    ShapeWidenThenUse,
		A:        "layout.set",
		B:        "agents.spawn",
		Crossing: "the hub stores the document verbatim because it 'does not interpret' it, and the desktop adopts it on hydration and respawns every stopped agent in it through window.electronAPI.spawnClaude — the LOCAL IPC door, which scrubs nothing. The bus's own agents.spawn refuses skipPermissions, an escalating permissionMode, a bypassing profile and caller-supplied mcpItemIds; all four arrived at the spawn anyway, from a caller that may not spawn at all.",
		ClosedBy: "layout.scrubAdoptedSpawnFields, applied to every NON-TRUSTED layout.set (the hub registers it through RegisterLocalIdent so the writer's identity is known), stripping exactly the four fields agents.spawn strips",
		Bearings: []Bearing{{
			Kind: BearsAtCallSite, On: "layout.set", Symbol: "scrubAdoptedSpawnFields",
			Entry: Site{"setScrubbed", hubLayoutFile},
			Chain: []Site{{"setScrubbed", hubLayoutFile}, {"scrubAdoptedSpawnFields", hubLayoutFile}},
		}},
	},
	{
		Name:     "a plugin pane's ${agentCwd} binding comes out of the shared layout document, so that document sets the plugin sandbox's own boundary",
		Shape:    ShapeWidenThenUse,
		A:        "layout.set",
		B:        "fs.read",
		Crossing: "the bus's per-plugin path confinement is the ONE guard that is per-caller rather than per-host, and its root set is bound from the pane the host is opening — a pane read out of the shared layout document. An agent with cwd '/' produced a pane token whose fsRoots were ['/'], and a volume root contains everything below it, so the plugin's fs.* capabilities then admitted every path on the machine. Neither call is wrong: layout.set writes an opaque document and PaneToken faithfully binds what the trusted host hands it.",
		ClosedBy: "expandScope refuses a bare-token binding that RESOLVES to a volume root (internal/plugin/manager.go isVolumeRoot), the narrowing the subpath branch already had",
		Bearings: []Bearing{{
			Kind: BearsOnGrantedRoots, On: "fs.read", Symbol: "isVolumeRoot",
			Entry: Site{"expandScope", hubPluginMgrFile},
		}},
	},
	{
		Name:     "replay.open cuts a worktree under a grant; replay.read keeps serving it after the grant is revoked",
		Shape:    ShapeWidenThenUse,
		A:        "replay.open",
		B:        "replay.read",
		Crossing: "replay.open is confined to workspaceRoots because it cuts a worktree from the repo at cwd; replay.read is excused on the grounds that containment is 'structural'. It was — but keyed by a CALLER-CHOSEN sessionId in a process-global map whose only eviction is an explicit close. Once the session stopped, fs.read on the repo was refused and a fresh replay.open on it was refused, and replay.read went on returning its bytes to anyone who knew the id — which agents.list and sessions.snapshots hand out while classified inert.",
		ClosedBy: "guardReplaySession in hubCapabilities.ts re-runs replay.open's own containment on the entry's recorded origin cwd before every read/diff/seek",
		Bearings: []Bearing{argBearing("guardReplaySession", "replay.read", desktopCapsFile)},
	},
	{
		Name:     "the capability plane refuses sessions.attachTerminal to a view token; the event plane delivered its entire output",
		Shape:    ShapeWidenThenUse,
		A:        "sessions.attachTerminal",
		B:        "pty.bytes.*",
		Crossing: "two authorization planes answering the same question differently. mayCall denies the method to a scoped tier; mayConsume read `cn.trusted || cn.scopeMethods != nil || …`, whose middle clause waved every topic through for any scoped user token. terminals.* is in neither scoped tier at all, so the event plane was the only door onto a terminal's screen — raw PTY bytes with the ring-buffer replay attaching deliberately restarts.",
		ClosedBy: "the event-topic registry (eventtopics.go) consulted by mayConsume via EventTopicSpec, whose DEFAULT IS CLOSED for a scoped user token and which now also filters the plugin arm — plus the enqueue-time admission filter, so a refused stream no longer even leaves a drop record to escape as pty.desync",
		// The registry is the mechanism, so the proof is the registry itself
		// saying that this topic is guarded by this pair's OTHER half.
		Bearings: []Bearing{{Kind: BearsInTopicRegistry, On: "pty.bytes.*", Symbol: "EventTopicSpec"}},
	},
	{
		Name:     "sessions.save writes the boot-restore document; the desktop's next launch respawns it through the LOCAL spawn door",
		Shape:    ShapeWidenThenUse,
		A:        "sessions.save",
		B:        "agents.spawn",
		Crossing: "layout.set's recorded pair, reached through a DIFFERENT writer that was never scrubbed. sessions.save stamps `timestamp: now` into <configDir>/sessions/<slug>.yaml, which makes it sessions[0]; useSessionLifecycle loads it on boot, migrateSessionData passes the modern format through as-is, and reconcileAgents{respawnStopped:true} hands every card claudemon no longer holds to respawnFromRecord — which forwards profileId, permissionMode, skipPermissions and mcpItemIds to window.electronAPI.spawnClaude, the LOCAL IPC door that scrubs nothing. capspec excused the method as a PATH question (\"the filename is derived from the session name by the provider's slug\") and nothing in either provider looked at what the document CONTAINS.",
		ClosedBy: "scrubBootDocumentAgents, applied unconditionally on both providers (cmd/brain/bootdoc.go and main/lib/bootDocumentScrub.ts) because caller identity does not reach a bus provider — stripping exactly the four fields internal/layout's scrubAdoptedSpawnFields strips, with the three lists held equal by a test",
		Bearings: []Bearing{argBearing("scrubBootDocumentAgents", "sessions.save", brainHandlersFile)},
	},
	{
		Name:     "layouts.save writes the same agents array into the layout template the Layouts menu restores",
		Shape:    ShapeWidenThenUse,
		A:        "layouts.save",
		B:        "agents.spawn",
		Crossing: "the third copy of the boot-restore shape: <configDir>/layouts/<slug>.yaml holds \"the caller's whole agents array\" and is restored from the Layouts menu into the same loadAgentsFromSession -> reconcileAgents -> respawnFromRecord path as a saved session. One document shape, three writers, and the composition record named one of them — which is precisely how a closed chain stays reachable through a second door.",
		ClosedBy: "scrubBootDocumentAgents on both providers, the same call the sessions.save pair is closed by",
		Bearings: []Bearing{argBearing("scrubBootDocumentAgents", "layouts.save", brainHandlersFile)},
	},
	{
		Name:     "push.subscribe records an outbound network sink; agents.sendMessage pulls the trigger that makes the host use it",
		Shape:    ShapeWidenThenUse,
		A:        "push.subscribe",
		B:        "agents.sendMessage",
		Crossing: "push.subscribe stores a row; a DIFFERENT subsystem (push.Watch -> onSnapshot -> sendOne) consults that row to issue POST <endpoint> with a VAPID header from the HOST's network position — Tailscale-reachable, loopback-reachable, cloud-metadata-reachable — for a tier holding no fetch, no exec, no fs and no config capability. The trigger is the un-blocked -> blocked edge, and the same triage tier holds agents.sendMessage and claude.approve, so it can drive an agent into and out of that state on demand. capspec's excuse reasoned entirely about what the ENDPOINT learns (\"the payload is encrypted to the subscription's own keys\") and never about what the HOST is made to do.",
		ClosedBy: "validatePushEndpoint (internal/push/endpoint.go): https only, and no loopback, private, link-local (169.254.169.254) or unique-local host — the shape a browser PushManager actually produces",
		Bearings: []Bearing{{
			Kind: BearsAtCallSite, On: "push.subscribe", Symbol: "validatePushEndpoint",
			Entry: Site{"RPCSubscribeAs", hubPushFile},
			Chain: []Site{{"RPCSubscribeAs", hubPushFile}, {"validatePushEndpoint", hubPushEndptFile}},
		}},
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
// The record above is eleven pairs somebody typed. The tier guard fires only when
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
//
// AND THE WRITING HAD THE SAME HOLE THE CLOSURES DID. A 200-character sentence
// was the entire bar, so a FALSE inert reason satisfied the forcing function
// exactly as well as a true one: injecting a method into unscopedByDecision and
// pairing it with plausible prose here left the suite green. Prose cannot be
// checked, so an inert claim now carries WITNESSES — facts about that method
// which are checked against source or against capspec's own tables, and which
// the claim's own sentence has to name. Nothing proves the absence of an
// interpreter; what a witness pins is the load-bearing, falsifiable part of the
// claim (this cwd is confined by THAT call; this method only undoes THAT one;
// these caller values are classified per-parameter). The claims that have no
// witness at all are a CLOSED, pinned set — three of them — so "no evidence" is
// a named exception rather than the default.

// WitnessKind is the closed vocabulary of checks an inert claim can rest on.
type WitnessKind string

const (
	// WitnessRecordedHalf — the method is a half of a recorded pair; the pair is
	// the record, and the claim is a cross-reference to it.
	WitnessRecordedHalf WitnessKind = "recorded-half"
	// WitnessGuarded — a [Bearing]: the confinement this claim rests on is a real
	// call naming this method.
	WitnessGuarded WitnessKind = "guarded"
	// WitnessNarrows — the method is the UNDO of another one: it can only shrink
	// what the named twin widens. Checked structurally (same namespace, a verb
	// pair from a closed table) and the twin must itself be considered, so
	// "it only removes things" cannot be claimed about an unrelated method.
	WitnessNarrows WitnessKind = "narrows"
	// WitnessTopicGuard — capspec's event registry makes this capability the
	// gate on a topic, so the claim "its output is governed where the output
	// lives" is checked against the registry.
	WitnessTopicGuard WitnessKind = "topic-guard"
	// WitnessParamsClassified — the claim rests on the per-parameter record:
	// every named param must carry a dangerous-kind decision in capspec's own
	// tables, and the sentence must name it.
	WitnessParamsClassified WitnessKind = "params-classified"
	// WitnessNone — no check exists. Admissible only for the pinned set in
	// composition_test.go, so adding one is a test failure by name.
	WitnessNone WitnessKind = "none"
)

// Witness is one check backing an inert claim.
type Witness struct {
	Kind WitnessKind
	// Bearing backs WitnessGuarded.
	Bearing *Bearing
	// Widens backs WitnessNarrows: the method this one is the undo of.
	Widens string
	// Topic backs WitnessTopicGuard.
	Topic string
	// Params backs WitnessParamsClassified.
	Params []string
}

// InertClaim is "considered, and it cannot be half of a composition" plus the
// evidence for it.
type InertClaim struct {
	// Reason answers the two shapes:
	//
	//	WRITE-THEN-INTERPRET  do the bytes this call writes end up somewhere another
	//	                      capability, the host, or a third-party interpreter
	//	                      reads as CONFIG, CODE, ARGV or POLICY?
	//	WIDEN-THEN-USE        does this call change state — a grant, a root set, a
	//	                      permission mode, an approval gate, a session — that
	//	                      some other guard CONSULTS?
	//
	// "It is read-only" is a real answer. "It is safe" is not: every half of
	// every pair above is safe.
	Reason string
	// Witnesses are the checks the reason rests on. Empty is the recorded-half
	// form (Reason must then be empty too).
	Witnesses []Witness
}

func guarded(b Bearing) Witness       { return Witness{Kind: WitnessGuarded, Bearing: &b} }
func narrows(widens string) Witness   { return Witness{Kind: WitnessNarrows, Widens: widens} }
func topicGuard(topic string) Witness { return Witness{Kind: WitnessTopicGuard, Topic: topic} }
func paramsClassified(params ...string) Witness {
	return Witness{Kind: WitnessParamsClassified, Params: params}
}

// noWitness is the explicit "nothing here is checkable" marker. Its members are
// pinned by name in the test; do not add one without reading that pin.
var noWitness = Witness{Kind: WitnessNone}

// recordedHalf is the cross-reference form: the method's composition IS a
// recorded pair.
var recordedHalf = InertClaim{}

// pathGuard is the fs confinement bearing: assertPathAllowed called with the
// method's own name, in the desktop provider.
func pathGuard(method string) Witness {
	return guarded(argBearing("assertPathAllowed", method, desktopCapsFile))
}

// gitCwdGuard is unscopedByDecision's own provider claim for the git block,
// re-used as a composition witness: the cwd these methods read/write in is the
// thing that could be aimed elsewhere, and guardGitCwd is what stops it.
func gitCwdGuard(method string) Witness {
	return guarded(argBearing("guardGitCwd", method, desktopCapsFile))
}

// compositionInert is the written record of "considered, and it cannot be half
// of a composition", with the evidence each sentence rests on.
var compositionInert = map[string]InertClaim{
	// ── recorded halves; listed for the guard's own completeness check ──────
	"fs.read":                 recordedHalf,
	"fs.write":                recordedHalf,
	"search.project":          recordedHalf,
	"providers.listModels":    recordedHalf,
	"layout.set":              recordedHalf,
	"layouts.save":            recordedHalf,
	"sessions.save":           recordedHalf,
	"agents.spawn":            recordedHalf,
	"agents.sendMessage":      recordedHalf,
	"claude.approve":          recordedHalf,
	"replay.open":             recordedHalf,
	"replay.read":             recordedHalf,
	"sessions.attachTerminal": recordedHalf,
	"push.subscribe":          recordedHalf,

	// ── reads that produce no durable state ────────────────────────────────
	"fs.readImage": {
		Reason:    "returns decoded image bytes to the caller and writes nothing. Its path is confined by the same assertPathAllowed('fs.readImage', …) call fs.read makes, and no interpreter sits downstream: nothing in the host re-reads an image as configuration",
		Witnesses: []Witness{pathGuard("fs.readImage")},
	},
	"fs.listEntries": {
		Reason:    "returns names and types under a root confined by assertPathAllowed('fs.listEntries', …). Composed with fs.write it is the shell-shaped pair the round-5 record already closes at the CONTAINMENT level (one predicate, 137 cases); it writes nothing itself and no guard consults its output",
		Witnesses: []Witness{pathGuard("fs.listEntries")},
	},
	"fs.listDir": {
		Reason:    "the same enumeration as fs.listEntries with a different result shape and the same assertPathAllowed('fs.listDir', …) confinement; identical reasoning",
		Witnesses: []Witness{pathGuard("fs.listDir")},
	},
	"fs.unwatch": {
		Reason:    "removes a watcher this caller installed — the undo of fs.watch, confined by the same assertPathAllowed('fs.unwatch', …) call. It can only ever SHRINK what fs.changed carries, and no guard consults the watcher set",
		Witnesses: []Witness{pathGuard("fs.unwatch"), narrows("fs.watch")},
	},
	"git.status": {
		Reason:    "runs `git status` in a cwd guardGitCwd('git.status', …) confines to the workspace roots, and returns text. Writes nothing; the porcelain output is not read as policy by anything",
		Witnesses: []Witness{gitCwdGuard("git.status")},
	},
	"git.log": {
		Reason:    "reads commit metadata out of a repo guardGitCwd('git.log', …) confines, and returns it. Writes nothing, changes no state, and no guard in the system consults commit history when deciding anything",
		Witnesses: []Witness{gitCwdGuard("git.log")},
	},
	"git.numstat": {
		Reason:    "reads per-file change counts for a commit range in a repo guardGitCwd('git.numstat', …) confines. Numbers to a UI: nothing is written, and nothing downstream reads the result as configuration or argv",
		Witnesses: []Witness{gitCwdGuard("git.numstat")},
	},
	"git.commitDiff": {
		Reason:    "reads one commit's patch text out of a repo guardGitCwd('git.commitDiff', …) confines, under the same result-path secret gate git.diff has, so it cannot return bytes fs.read would refuse. Writes nothing",
		Witnesses: []Witness{gitCwdGuard("git.commitDiff")},
	},
	"git.commitNumstat": {
		Reason:    "reads one commit's change counts in a repo guardGitCwd('git.commitNumstat', …) confines — the per-commit twin of git.numstat, with the same absence of a writer and of a downstream interpreter",
		Witnesses: []Witness{gitCwdGuard("git.commitNumstat")},
	},
	"files.upload": {
		Reason:    "writes caller bytes to a FRESHLY CREATED, hub-named 0600 file under os.TempDir()/workspacer-uploads — a directory nothing in the host reads as config, code, argv or policy, with the caller's `name` param reduced to its allowlisted image/pdf extension (its per-param decision is on the record) so no executable class lands. WRITE-THEN-INTERPRET: the only downstream reader is an agent, and only if a caller also names the path via agents.sendMessage — which is the tier's one AcceptedIn pair, whose excuse (the agent's own tool approvals gate what a message makes it read) covers an uploaded image exactly as it covers any pre-existing host path a message names. WIDEN-THEN-USE: it changes no grant, root set, permission mode, approval gate or session, and no guard consults the upload directory; the returned path is information, not authority",
		Witnesses: []Witness{paramsClassified("name")},
	},
	"claude.sessionsForDir": {
		Reason:    "lists claudemon's known sessions for a directory. Read-only, its `cwd` carries a per-parameter decision on the record, and the ids it returns are already handed out by agents.list and sessions.snapshots",
		Witnesses: []Witness{paramsClassified("cwd")},
	},
	"claude.handoffBrief": {
		Reason:    "renders a deterministic handoff brief into ~/.workspacer/handoffs/ and returns its path. The successor agent's composer is PRE-FILLED with 'read this file' rather than instructed by it, and the file is prose, not argv — the interpreter is a human reading a chat box. Its argv/profile fields come from the handoff builder, not the caller. NOTHING HERE IS MACHINE-CHECKED: the claim is about what a human does with the text",
		Witnesses: []Witness{noWitness},
	},
	"claude.handoffAgentBrief": {
		Reason:    "the per-agent variant of claude.handoffBrief; same builder, same output location, same reasoning. NOTHING HERE IS MACHINE-CHECKED either, for the same reason: the interpreter is a human reading a chat box",
		Witnesses: []Witness{noWitness},
	},
	"replay.diff": {
		Reason:    "reads a diff out of a worktree replay.open cut. Its containment is the recorded replay.open→replay.read pair's, re-run per call by guardReplaySession('replay.diff', …)",
		Witnesses: []Witness{guarded(argBearing("guardReplaySession", "replay.diff", desktopCapsFile))},
	},
	"replay.seek": {
		Reason:    "moves a cursor inside a replay session, behind the same guardReplaySession('replay.seek', …) re-containment; no bytes leave the worktree that replay.read would not also return",
		Witnesses: []Witness{guarded(argBearing("guardReplaySession", "replay.seek", desktopCapsFile))},
	},
	"library.list": {
		Reason:    "enumerates prompt/skill markdown under a directory derived from a cwd assertPathAllowed('library.list', …) confines, with guardLibraryFile('library.list', …) over the item paths themselves. The items are inserted into a composer for a human to send, not executed",
		Witnesses: []Witness{guarded(argBearing("assertPathAllowed", "library.list", desktopCapsFile))},
	},

	// ── writes whose bytes have no interpreter ─────────────────────────────
	"library.save": {
		Reason:    "writes a .md prompt/skill into the library directory guardLibraryCwd('library.save', …) confines. The bytes become CHAT TEXT a human sends, never argv or config — and the one place a library item IS interpreted (a skill file) is generated by the host, not by this call",
		Witnesses: []Witness{guarded(argBearing("guardLibraryCwd", "library.save", desktopCapsFile))},
	},
	"library.remove": {
		Reason:    "deletes one library item, inside the same guardLibraryCwd('library.remove', …) confinement — the undo of library.save. Removal cannot introduce an interpreter, and nothing consults the item set as policy",
		Witnesses: []Witness{guarded(argBearing("guardLibraryCwd", "library.remove", desktopCapsFile)), narrows("library.save")},
	},
	"sessions.delete": {
		Reason:    "deletes a saved session document. It can only remove a boot-restore document, never add one — the ADD direction is sessions.save, whose recorded pair with agents.spawn is where this document's composition lives",
		Witnesses: []Witness{narrows("sessions.save")},
	},
	"layouts.delete": {
		Reason:    "deletes a saved layout template; same direction, same reasoning as sessions.delete, with layouts.save as the widening twin",
		Witnesses: []Witness{narrows("layouts.save")},
	},
	"sessions.load": {
		Reason:    "reads a saved session document back; its `filename` carries a per-parameter decision on the record. The document's dangerous direction is its WRITER (the recorded pair); reading it hands the caller bytes it could have written",
		Witnesses: []Witness{paramsClassified("filename")},
	},
	"sessions.transcript": {
		Reason:    "returns a session's transcript text for a `cwd` that carries a per-parameter decision on the record. Read-only, and the transcript is rendered, never executed",
		Witnesses: []Witness{paramsClassified("cwd")},
	},
	"config.save": {
		Reason: "writes config.yaml, which IS re-read as argv by the host — and that is exactly why every such key (agents.binaries, claude.profiles, terminal.shell, terminal.shells, editor.terminalCommand, scripts, updates) is classified per-parameter in capspec and STRIPPED from a bus write by dropHostTrusted, held equal to contracts/host-trusted-config-cases.json. The composition is real, it is closed per-key rather than per-pair, and the per-key record is the one that cannot drift silently",
		Witnesses: []Witness{paramsClassified(
			"agents.binaries", "claude.profiles", "terminal.shell", "terminal.shells",
			"editor.terminalCommand", "scripts", "updates",
		)},
	},
	// ── jobs.* — the hub's job system (internal/jobs) ──────────────────────
	// jobs.upsert/jobs.run are the recorded pair below; the rest are gated by
	// the same jobsTrusted identity check, invoked with each method's own name.
	"jobs.upsert": recordedHalf,
	"jobs.run":    recordedHalf,
	"jobs.propose": {
		Reason:    "the agent-facing half of jobs.upsert, deliberately weaker: it can only CREATE, what it creates is forced disabled and stamped proposedBy, and jobs.run refuses a stamped row — so the argv it persists cannot execute until a trusted caller writes that row back with the stamp cleared. Gated by jobsTrusted like every other jobs.* method (an operator token passes, plugin and view/triage tokens do not); the restraint that matters here is not the identity gate but the method — the MCP facade hands agents a tool for this and none for jobs.upsert",
		Witnesses: []Witness{guarded(argBearing("jobsTrusted", "jobs.propose", hubMainFile))},
	},
	"jobs.list": {
		Reason:    "returns the stored specs — which DISCLOSE shell commands and agent prompts, which is why jobsTrusted refuses every non-host caller — but writes nothing and executes nothing",
		Witnesses: []Witness{guarded(argBearing("jobsTrusted", "jobs.list", hubMainFile))},
	},
	"jobs.remove": {
		Reason:    "deletes a stored job and its history behind the same jobsTrusted gate — the undo of jobs.upsert; it cannot introduce argv, only retire it",
		Witnesses: []Witness{narrows("jobs.upsert"), guarded(argBearing("jobsTrusted", "jobs.remove", hubMainFile))},
	},
	"jobs.history": {
		Reason:    "returns run records (shell output tails included — disclosure, which is what jobsTrusted gates) for a stored job id; writes nothing and executes nothing",
		Witnesses: []Witness{guarded(argBearing("jobsTrusted", "jobs.history", hubMainFile))},
	},

	"claude.profiles.add": {
		Reason:    "persists a profile whose configDir becomes CLAUDE_CONFIG_DIR and whose extraArgs become argv — both classified per-parameter, alongside mcpItemIds. The interpreter is real, and it is closed by scrubProfileBypass at write time on BOTH providers, with agents.spawn refusing a bypassing profileId from a bus caller as the second half",
		Witnesses: []Witness{paramsClassified("configDir", "extraArgs", "mcpItemIds")},
	},
	"claude.profiles.update": {
		Reason:    "the same configDir / extraArgs / mcpItemIds fields through the same scrub as claude.profiles.add, and the same per-parameter decisions",
		Witnesses: []Witness{paramsClassified("configDir", "extraArgs", "mcpItemIds")},
	},
	"claude.profiles.remove": {
		Reason:    "removes a profile — the undo of claude.profiles.add. It cannot introduce a configDir or extraArgs; a spawn naming a removed profile falls back to the default",
		Witnesses: []Witness{narrows("claude.profiles.add")},
	},
	"notifications.post": {
		Reason:    "renders a title/body into an OS notification and an in-app card; its one value the host acts on is `url`, which carries a per-parameter decision. Text to a human otherwise: no argv, no file, and nothing consults the notification set",
		Witnesses: []Witness{paramsClassified("url")},
	},
	"push.unsubscribe": {
		Reason:    "drops a push subscription, proven by possession of the subscription's own `endpoint` auth secret — the undo of push.subscribe, whose recorded pair carries that sink's composition. Removal only",
		Witnesses: []Witness{narrows("push.subscribe"), paramsClassified("endpoint")},
	},
	"push.revoke": {
		Reason:    "drops another credential's push subscriptions by `id` — a revocation, i.e. the direction that can only narrow what push.subscribe widened",
		Witnesses: []Witness{narrows("push.subscribe"), paramsClassified("id")},
	},

	// ── state changes no other guard consults ──────────────────────────────
	"claude.setModel": {
		Reason:    "switches the model of a running agent over the same live-switch endpoint claude.setEffort uses, with `effort` the classified value riding beside it. No guard anywhere reads the model; the permission mode, which several do read, is a different method",
		Witnesses: []Witness{paramsClassified("effort")},
	},
	"claude.setEffort": {
		Reason:    "switches a running agent's reasoning `effort`, classified per-parameter. Like the model, effort is a parameter of generation that no guard anywhere consults — it is not the permission mode, which several do",
		Witnesses: []Witness{paramsClassified("effort")},
	},
	"claude.signal": {
		Reason:    "sends an interrupt/stop signal to a running agent. It can only ever STOP work; nothing consults it. NOTHING HERE IS MACHINE-CHECKED: the method carries no classified value and no guard names it, so this sentence is the whole of the evidence",
		Witnesses: []Witness{noWitness},
	},
	"claude.gate": {
		Reason:    "parks a tool call for human approval — it only ADDS a gate, and its `on` value is classified per-parameter. Removing one is claude.approve, the recorded half this method is the undo of",
		Witnesses: []Witness{narrows("claude.approve"), paramsClassified("on")},
	},
	"claude.answer": {
		Reason:    "answers an agent's question prompt: `text`, `answers` and `option` are each classified as PTY bytes, the same primitive sessions.terminalInput carries. Gated by the agent's own approvals exactly as agents.sendMessage is — and that pair (agents.sendMessage + claude.approve) is recorded and accepted for the triage tier, which is where this method's composition risk already lives",
		Witnesses: []Witness{paramsClassified("text", "answers", "option")},
	},
	"claude.setPermissionMode": {
		Reason: "changes the `mode` a running agent runs in, which IS state a later guard consults — and it is closed by the shared escalation allow-list, called as assertNoPermissionBypass('claude.setPermissionMode', mode), the same clamp agents.spawn applies",
		Witnesses: []Witness{
			guarded(argBearing("assertNoPermissionBypass", "claude.setPermissionMode", desktopCapsFile)),
			paramsClassified("mode"),
		},
	},
	"sessions.terminalInput": {
		Reason:    "types bytes into a session's PTY — `data` and `bytesB64`, both classified as exactly that. Its OUTPUT side (sessions.attachTerminal / pty.bytes.*) is a recorded pair; the input side reaches a shell that is already running as the user, which is what terminals.create's own allow-list governs",
		Witnesses: []Witness{paramsClassified("data", "bytesB64")},
	},
	"terminals.create": {
		Reason:    "starts a shell: `shell` is argv[0] and `cwd` is where it runs, both classified per-parameter, and argv[0] is closed by the shell allow-list (lib/shellAllowlist.ts + cmd/brain/shellallow.go, one list held equal by a corpus). The config keys that could redirect that argv are stripped by dropHostTrusted — the two halves of that chain are classified where they live",
		Witnesses: []Witness{paramsClassified("shell", "cwd")},
	},
	"terminals.open": {
		Reason:    "opens a VISIBLE terminal pane and runs `command` in the host's default login shell (no caller argv[0]): `cwd` and `command` are classified per-parameter, and the only thing that ever acts on either is the RENDERER the desktop forwards to — a headless brain registers no such method, so there is no second interpreter for a later guard to worry about. The line runs under the opened shell's own tool/PTY rules, exactly as sessions.terminalInput's typed bytes do",
		Witnesses: []Witness{paramsClassified("cwd", "command")},
	},
	"git.stage": {
		Reason:    "stages paths inside a repo guardGitCwd('git.stage', …) confines. The index is git's own state; no capability here reads it as policy, and the commit that consumes it is git.commit",
		Witnesses: []Witness{gitCwdGuard("git.stage")},
	},
	"git.unstage": {
		Reason:    "removes paths from the index of a repo guardGitCwd('git.unstage', …) confines — the undo of git.stage, so it can only ever shrink what a later git.commit records, and nothing reads the index as policy",
		Witnesses: []Witness{gitCwdGuard("git.unstage"), narrows("git.stage")},
	},
	"git.commit": {
		Reason:    "records a commit in a repo guardGitCwd('git.commit', …) confines — its one classified caller value is that `cwd`. A commit message is not interpreted by anything in this system, and hooks in .git/hooks are refused by the secret gate that covers .git",
		Witnesses: []Witness{gitCwdGuard("git.commit"), paramsClassified("cwd")},
	},
	"git.push": {
		Reason:    "publishes commits to the remote of a repo guardGitCwd('git.push', …) confines. The remote URL and every exec-adjacent git config key are classified per-parameter (round 4's finding: 'the fix named one exec key; git config has a dozen'), which is where that composition is closed",
		Witnesses: []Witness{gitCwdGuard("git.push")},
	},
	"git.diff": {
		Reason:    "reads file contents out of the repo at a cwd guardGitCwd('git.diff', …) confines, and its result-path secret gate is the recorded fs.write→search.project pair's closer applied to the same read-set invariant",
		Witnesses: []Witness{gitCwdGuard("git.diff")},
	},
	"fs.watch": {
		Reason:    "installs a change watcher on a path assertPathAllowed('fs.watch', …) confines. Its OUTPUT is the fs.changed topic, and that is where its composition lives: the event registry names this capability as that topic's gate, so a credential refused fs.watch is refused the change feed it produces. The call itself writes nothing and no guard consults the watcher set",
		Witnesses: []Witness{pathGuard("fs.watch"), topicGuard("fs.changed")},
	},
}

// CompositionInertReason returns the written reason a capability was found
// unable to be half of a composition, and whether one is recorded. An empty
// reason with ok=true means the method IS a recorded half and needs no separate
// statement.
func CompositionInertReason(method string) (string, bool) {
	c, ok := compositionInert[method]
	return c.Reason, ok
}

// CompositionInertClaim returns the full claim, evidence included.
func CompositionInertClaim(method string) (InertClaim, bool) {
	c, ok := compositionInert[method]
	return c, ok
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
// CompositionActorsForTest exposes compositionActors to the package's own tests,
// which need it to tell a dispatch switch from a handler. Not part of the public
// surface beyond that.
func CompositionActorsForTest() []string { return compositionActors() }

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
