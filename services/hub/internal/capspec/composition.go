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
		ClosedBy: "eventTopicGuards + EventTopicCapability, consulted by mayConsume for scoped user tokens: a topic that carries a capability's output requires that capability",
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
