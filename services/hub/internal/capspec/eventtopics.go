package capspec

import (
	"sort"
	"strings"
)

// THE EVENT PLANE. Everything else in this package governs the CALL verb — who
// may invoke a method and with what arguments. The pub/sub plane governs
// SUBSCRIBE and PUBLISH, and until this file existed it was governed by a
// two-row DENYLIST whose default was OPEN:
//
//	mayConsume: if guarded { return MatchesAny(...) }; return true
//
// Two rows, twenty-three topics. A matrix run against the real bus with the real
// tiers published all twenty-five enumerated topics and a `view` token received
// twenty-three of them. Among them:
//
//   - pty.desync, naming the sessionId of a pty.bytes stream mayConsume had just
//     REFUSED this same connection — the drop bookkeeping for a guarded topic
//     escaping as its own event;
//   - pty.exit, the end-of-stream signal of the same guarded stream;
//   - agent.statusline, carrying model, cost_usd, context_used_pct and
//     five_hour_pct for a session the fleet-visibility rule was hiding, on the
//     one publish path that never asked vis.visible;
//   - plugin.log, one verbatim unredacted line of a sidecar's stdout/stderr,
//     whose environment carries WKS_SETTINGS with secret plugin settings in
//     PLAINTEXT ("Deliberately UNREDACTED", settings.go).
//
// A denylist cannot be completed by adding rows, because nothing forces the next
// row. So the default is inverted here: for a scoped user token an UNCLASSIFIED
// topic is refused, and TestEveryPublishedTopicIsClassified enumerates every
// publish site in the repo and fails on a topic this table does not name. A new
// topic now fails until classified, exactly the way a new capability does.
//
// THE RULE WAS ALSO INCOMPLETE, not just its table. The old rule — "a topic that
// carries a capability's OUTPUT requires that capability" — cannot classify a
// topic whose payload NO capability returns. plugin.log is the proof: grep the
// brain handlers and hubCapabilities.ts for any method that returns sidecar
// logs and there is none, so no `guardedBy` row could ever cover it, and under a
// rule with only that disposition it would stay open forever by construction.
// Eleven of the twenty-three view-reachable topics were in that state.
//
// Hence three dispositions, mirroring the call plane's own shape (PathParam /
// unscopedByDecision / inertMethods):
//
//	TopicGuardedBy     the topic carries a capability's output; holding that
//	                   capability is the price of receiving it.
//	TopicHostOnly      no capability returns this payload AND it is not fit for a
//	                   scoped tier. Trusted connections only. This is the
//	                   disposition the old rule could not express.
//	TopicOpenByDecision every tier may receive it, and here is why — the same
//	                   "decision on the record" form as unscopedByDecision.
//
// PLUGINS. A plugin's event reach is its manifest `consumes`, declared at
// install and shown in the consent dialog. That is a real answer for a topic
// nobody else classified — a plugin-defined topic like `example.clock.tick` or
// the rules engine's `command.*` is not in this table and never will be — but it
// was NOT an answer for pty.bytes.*: a plugin with ZERO capabilities and
// `consumes: ["pty.bytes.*", "fs.changed"]` was refused sessions.attachTerminal
// and fs.watch on the call plane and handed both capabilities' entire output on
// the event plane, while the consent dialog rendered those two lines at
// severity=normal. So for a topic this table DOES name, the manifest is a filter
// and not a grant: a guarded topic needs the capability, and a host-only topic
// is refused outright.
//
// PUBLISH DIRECTION. Every topic in this table is published by the HOST — brain,
// the hub itself, the supervisor, the plugin manager, the desktop main process.
// A non-trusted connection publishing one is forging host state, and two proven
// chains did exactly that: a forged layout.changed carried the four
// spawn-escalation fields layout.set scrubs, straight into the renderer's live
// agent state; a forged agent.snapshot drove the phone's "needs you" Web Push
// with attacker-chosen text AND suppressed the genuine alert for that session by
// moving push's state machine past the edge it fires on. Classification is
// therefore also an OWNERSHIP claim, enforced by mayPublish.

// EventTopicDisposition is the closed vocabulary of answers to "may a
// non-trusted credential receive this topic".
type EventTopicDisposition string

const (
	// TopicGuardedBy — the payload is a capability's output; that capability is
	// the price of admission.
	TopicGuardedBy EventTopicDisposition = "guarded-by-capability"
	// TopicHostOnly — no capability returns this payload and it is not fit for
	// any scoped tier. Trusted connections only.
	TopicHostOnly EventTopicDisposition = "host-only"
	// TopicOpenByDecision — every tier may receive it, with the reason recorded.
	TopicOpenByDecision EventTopicDisposition = "open-by-decision"
)

// EventTopic is one classified topic.
//
// Pattern uses the bus topic syntax: an exact type, or a trailing "*" matching
// any suffix. Method is set only for TopicGuardedBy; Reason is required for the
// other two, and is the whole content of the row.
type EventTopic struct {
	Pattern     string
	Disposition EventTopicDisposition
	Method      string
	Reason      string
	// Publisher names the capability whose PROVIDER emits this topic. It answers
	// a DIFFERENT question from Method: Method is "who may HEAR this",
	// Publisher is "who may SAY it".
	//
	// They coincide on the rows a headless provider emits — the provider of
	// sessions.attachTerminal IS the terminal, so its pty.bytes.<id> is the
	// stream and not a forgery — and they deliberately do not in general.
	// plugin.log is guarded on the consume side and published only by the hub's
	// own sidecar supervisor, so no capability answers for it and its Publisher
	// is empty. An empty Publisher is a real answer, not an unfilled field: it
	// means "the host says this and nobody may say it on the host's behalf",
	// which is the correct reading for layout.changed (whose forged form
	// carries the four spawn-escalation fields layout.set scrubs) and for
	// plugin.settings.changed.
	//
	// Read by the bus's mayPublish, which is the ONLY widening in the provider
	// tier: a non-trusted connection may publish a classified topic exactly
	// when it registered the capability named here.
	Publisher string
}

// eventTopics is the registry. Every topic any publish site in this repo emits
// must appear here — see TestEveryPublishedTopicIsClassified, which scans the
// publish call sites rather than this table, because a table that validates only
// itself is the hand-maintained record this whole effort keeps replacing.
var eventTopics = []EventTopic{
	// ---- the PTY family -------------------------------------------------
	// One stream, three topics. Round 6 guarded the first by name and left its
	// two siblings open, which is what "fix the leg, not the class" looks like.
	{
		Pattern:     "pty.bytes.*",
		Disposition: TopicGuardedBy,
		Method:      "sessions.attachTerminal",
		// The provider of the terminal is the thing the bytes come OUT of:
		// cmd/brain/terminal.go publishes this from the PTY it owns.
		Publisher: "sessions.attachTerminal",
		Reason:    "the session's raw PTY byte stream with the ring-buffer replay — as sessions.attachTerminal's own capspec reason says, \"what it grants is the OUTPUT side of sessions.terminalInput\"",
	},
	{
		Pattern:     "pty.exit",
		Disposition: TopicGuardedBy,
		Method:      "sessions.attachTerminal",
		Reason:      "end-of-stream for the same guarded stream: it names a sessionId and tells a credential that may not watch the terminal exactly when that terminal died",
	},
	{
		// The VISIBLE-terminal request. terminals.open does not start a process:
		// it asks a CLIENT to open a terminal pane so the user can watch one.
		// The desktop provider does that with emitToRenderer over IPC, which
		// never touches the bus; the headless brain has no renderer, so this
		// topic IS its only way to be answered (cmd/brain/visibleterm.go).
		//
		// Guarded by the method that publishes it, because the payload is a HOST
		// COMMAND LINE — `command`, plus the cwd it runs in — and receiving one
		// is the same disclosure as composing one. capspec's own entry for
		// terminals.open says that line executes inside the host's default login
		// shell exactly as a user-typed command would; a view or triage token
		// holds neither the method nor, therefore, the feed.
		Pattern:     "facade.openTerminal",
		Disposition: TopicGuardedBy,
		Method:      "terminals.open",
		// Method and Publisher are the same name here for a reason worth
		// stating: this topic IS how the headless provider answers
		// terminals.open (cmd/brain/visibleterm.go — it has no renderer to emit
		// to over IPC), so whoever answers the call is by construction whoever
		// emits the event.
		Publisher: "terminals.open",
		Reason:    "carries the cwd and the shell COMMAND LINE an agent asked to be run in a visible pane, so receiving it discloses exactly what sending it composes. It is the headless provider's only delivery path (the desktop emits the identical payload to its renderer over IPC and never publishes it), which is why the topic exists at all rather than the capability simply returning ok",
	},
	{
		Pattern:     "pty.desync",
		Disposition: TopicGuardedBy,
		Method:      "sessions.attachTerminal",
		Reason:      "synthesized from a DROPPED pty.bytes.<id> topic, so its payload is the identity of a stream mayConsume refused this same connection — the guarded topic's own bookkeeping, leaking as a side channel. The bus also stops recording a desync for a stream the connection may not consume; both halves are needed, since the row alone would still cost the denied subscriber its buffer",
	},

	// ---- filesystem ------------------------------------------------------
	{
		Pattern:     "fs.changed",
		Disposition: TopicGuardedBy,
		Method:      "fs.watch",
		Reason:      "a change feed on a path, republished to every subscriber rather than to the caller that asked for it — an activity oracle on files whose contents may still be unreadable. fs.watch is what installs the watcher",
	},

	// ---- the fleet feed --------------------------------------------------
	{
		Pattern:     "agent.statusline",
		Disposition: TopicGuardedBy,
		Method:      "sessions.snapshot",
		// status_line is merged into the session snapshot, so the provider of
		// the snapshot is the publisher of this (cmd/brain/events.go).
		Publisher: "sessions.snapshot",
		Reason:    "status_line is merged into the session snapshot (cmd/brain/store.go), so this is sessions.snapshot's output arriving by event. Guarding it is only half the fix: every OTHER fleet read runs through registry.visibleSnapshots / vis.visible and runStatusLines published unconditionally, disclosing the id, model, cost_usd and rate-limit state of a session the layout deliberately hides — and sessions.snapshot(id) is view-callable and unfiltered by id, so the leaked id completed the read. cmd/brain now applies the same visibility rule to this publish",
	},
	{
		Pattern:     "agent.snapshot",
		Disposition: TopicOpenByDecision,
		// The fleet feed, published by whoever provides the fleet read
		// (cmd/brain/main.go, off the same store). Open on the CONSUME side and
		// still owned on the PUBLISH side — internal/push treats this topic as
		// authoritative input, so a forged one puts attacker-chosen text on a
		// phone's lock screen AND suppresses the genuine alert. Open to hear is
		// not open to say.
		Publisher: "sessions.snapshots",
		Reason:    "the fleet feed the view tier exists for — /m, the web renderer and the TUI are all projections of it, and sessions.snapshot / sessions.snapshots are in the view tier already, so the event discloses nothing the call plane withholds. It is filtered by the SAME fleet-visibility rule as those reads before publication (cmd/brain/events.go store.onChange -> vis.visible)",
	},
	{
		Pattern:     "agent.state_changed",
		Disposition: TopicOpenByDecision,
		Reason:      "{sessionId, hookEvent, mode, cwd} — a strict subset of the agent.snapshot the same tier already receives, and the wake signal every remote client needs to stay live. Guarding it would take the fleet feed away from the tier it was built for while disclosing nothing extra",
	},
	// command.* — UI-navigation requests, published by the MCP facade's ui
	// tools (cmd/mcp/ui.go, triage tier and up) and consumed by the desktop
	// renderer (useUiCommands.ts). Plugins publish the same family under their
	// manifest emits, which is a filter this table does not govern; these rows
	// exist because a HOST component now publishes them with literals the
	// scanner reads. Open by decision: a command event is a REQUEST carrying
	// only its own arguments (a session id, a pane type, a URL) — it discloses
	// no host state, and the danger direction (who may PUBLISH one) is bounded
	// for non-trusted connections by mayPublish + manifest emits exactly as
	// before these rows existed.
	{
		Pattern:     "command.focus_agent",
		Disposition: TopicOpenByDecision,
		Reason:      "carries only the session id to focus — an id the view tier already receives on every agent.snapshot. The consumer is the trusted desktop renderer; a scoped receiver learns nothing it couldn't already read",
	},
	{
		Pattern:     "command.open_pane",
		Disposition: TopicOpenByDecision,
		Reason:      "carries a pane type plus optional cwd/url chosen by the publisher — a navigation request, not host state. Receiving it discloses only that navigation was requested",
	},
	{
		Pattern:     "command.open_plugin",
		Disposition: TopicOpenByDecision,
		Reason:      "carries an installed plugin's pane type, which the unauthenticated /plugins projection already lists",
	},
	{
		Pattern:     "command.open_spawn_dialog",
		Disposition: TopicOpenByDecision,
		Reason:      "carries an optional directory to pre-fill in the New Agent dialog. Opening the dialog spawns nothing — the spawn itself still goes through agents.spawn and its clamps",
	},
	{
		Pattern:     "command.open_guide",
		Disposition: TopicOpenByDecision,
		Reason:      "carries no arguments — a request to open the built-in guide pane. Receiving it discloses only that navigation was requested; anything the guide then does rides its own session's scoped token",
	},
	{
		Pattern:     "command.run_action",
		Disposition: TopicOpenByDecision,
		Reason:      "carries a keyboard-action id (plus an optional digit) from the desktop's ACTION_REGISTRY — pane zoom/swap, tab and agent navigation, panel toggles: the command layer's vocabulary as a bus event. The consumer is the trusted desktop renderer, which validates the id against the registry and REFUSES the decision verbs (approve/deny) — an approval must ride a scoped facade tool, never a published event. Receiving the event discloses only that navigation was requested",
	},
	// hub.peer.* — federation-link reachability (internal/federation). The
	// tombstone UX depends on every client hearing these: a peer going dark
	// must render as "hub unreachable, last seen …", not as agents silently
	// vanishing.
	{
		Pattern:     "hub.peer.connected",
		Disposition: TopicOpenByDecision,
		Reason:      "carries only the peer's configured name. The federated fleet events themselves are open to the view tier (agent.snapshot et al.), so the fact that a link to a named machine came up discloses strictly less than the feed it enables",
	},
	{
		Pattern:     "hub.peer.disconnected",
		Disposition: TopicOpenByDecision,
		Reason:      "peer name + last-seen timestamp. The signal every client needs to tombstone that hub's sessions instead of letting them silently vanish — which reads to a user as 'my agent died'",
	},
	// node.state_changed — remote worker node reachability (internal/nodes).
	// The same tombstone argument as hub.peer.*, one step further: a node can
	// be off ON PURPOSE, and a client that cannot hear this renders a machine
	// that is one button away from working as one that has died.
	{
		Pattern:     "node.state_changed",
		Disposition: TopicOpenByDecision,
		Reason:      "carries the same nodes.NodeView the view tier already reads from nodes.list, plus the state it came from: a node id, a label, one of four states, two timestamps, a sentence of detail, a wakeable bit and a failed-wake count. It is an allowlist PROJECTION of the registry record, so the credential-bearing half — the cloud API token, the file holding it, the app name, the machine id, the endpoint — is absent by construction and not by a strip list. Receiving it discloses strictly nothing beyond the read the same tier holds; what it buys is that a wake shows progress instead of a spinner. Publishing it is the hub's own (the topic is refused to every non-trusted publisher by the publish-ownership half of this registry), which matters because a forged \"available\" would send a client typing at a machine that is not there",
	},
	{
		Pattern:     "layout.changed",
		Disposition: TopicOpenByDecision,
		Reason:      "the accepted shared layout document, whose read (layout.get) is in the view tier and classified inert. The DANGER on this topic was never disclosure but forgery — a non-trusted publisher could put the four spawn-escalation fields layout.set scrubs into a document every client adopts verbatim — and that is closed by the publish-ownership half of this registry, not by withholding it",
	},
	{
		Pattern:     "workflow.started",
		Disposition: TopicOpenByDecision,
		Reason:      "run name, phases, agents and the absolute cwd — and that cwd is already in the view-reachable agent.snapshot for the same session, so the topic adds a run label to a disclosure the tier already has. The workflow feed is what Mission Control's remote views render",
	},
	{
		Pattern:     "workflow.completed",
		Disposition: TopicOpenByDecision,
		Reason:      "the completion half of workflow.started: status, durationMs, token and tool-call totals for a run whose cwd the tier already sees on agent.snapshot",
	},
	{
		Pattern:     "workflow.failed",
		Disposition: TopicOpenByDecision,
		Reason:      "the failure half of workflow.started, same payload shape and the same disclosure as the completion one",
	},
	{
		Pattern:     "workflow.agent.finished",
		Disposition: TopicOpenByDecision,
		Reason:      "per-agent roll-up of a run — label, model, status, tokens, toolCalls, phaseTitle. Model and token counts for a visible session are already in that session's snapshot and statusline",
	},
	{
		Pattern:     "library.changed",
		Disposition: TopicOpenByDecision,
		Reason:      "the payload is EMPTY — a bare \"refetch\" signal, published with no data field at all (libraryService.ts). library.list is in neither scoped tier, so a receiver learns only that something changed and cannot follow it; there is nothing here to withhold. If this topic ever grows a payload it must be reclassified, which is why the reason names the emptiness rather than the tier",
	},

	{
		Pattern:     "notify.post",
		Disposition: TopicHostOnly,
		Reason:      "an in-app notification: free-text title/body from whichever publisher raised it — a plugin's post, or a hub job failure whose body is a shell-output/error tail (internal/jobs). No capability returns notification feeds, and the bodies are written for the DESKTOP notification center, not for scoped tiers; phones get their alerts through the Web Push watcher, which never rides this topic. Withholding it from view/triage preserves exactly the pre-classification behaviour (an unclassified topic was refused to scoped tokens)",
	},

	// ---- the sidecar / plugin control plane ------------------------------
	// No capability returns any of these. Under the old "mirrors a capability"
	// rule they were unclassifiable and therefore permanently open; TopicHostOnly
	// is the disposition that exists for exactly this.
	{
		Pattern:     "plugin.log",
		Disposition: TopicHostOnly,
		Reason:      "one VERBATIM, unredacted line of a sidecar's stdout/stderr. No capability anywhere returns sidecar logs, so no guardedBy row could ever cover it — this topic is the reason this disposition exists. The leak vehicle is concrete: settingsEnvJSON puts secret plugin settings into the sidecar's WKS_SETTINGS environment in PLAINTEXT and says so, and a view token subscribed to \"*\" was observed receiving a sidecar's stderr carrying GITHUB_TOKEN=ghp_…. Emission is bounded today (only `workspacer plugin dev` calls SetStreamSidecarLogs(true)) but the reach was not",
	},
	{
		Pattern:     "sidecar.*",
		Disposition: TopicHostOnly,
		Reason:      "statusData{Name, State, PID, Err}: a host process id and the raw spawn/exec error text, which routinely carries absolute paths and argv. Process-supervision state of the host is operator business; no capability exposes it and no scoped tier has any use for it",
	},
	{
		Pattern:     "plugin.loaded",
		Disposition: TopicHostOnly,
		Reason:      "the whole Manifest — install argv and source, server command/args, and every declared filesystem path SCOPE, i.e. a map of what each sidecar may reach. A reader of this topic knows where to aim the next chain. GET /plugins currently serves the same thing without a guard(); that route is a separate bug and fixing it must not require re-deciding this topic, so the topic is decided on its own payload",
	},
	{
		Pattern:     "plugin.unloaded",
		Disposition: TopicHostOnly,
		Reason:      "the other edge of plugin.loaded — which sidecar just stopped being supervised. Same plane, same audience",
	},
	{
		Pattern:     "plugin.settings.changed",
		Disposition: TopicHostOnly,
		Reason:      "secrets ARE redacted to __WKS_SECRET__ here, but every NON-secret value (endpoints, org/repo names, absolute paths) is verbatim, and the equivalent READ — /plugins/settings — is guard()ed to the host token. Two planes disagreeing about one document is the exact defect this file exists to end, and the call plane's answer is the stricter one",
	},
	{
		Pattern:     "plugin.sandboxed",
		Disposition: TopicHostOnly,
		Reason:      "which OS confinement mechanism a sidecar got. The confinement inventory of the host is not fleet state",
	},
	{
		Pattern:     "plugin.sandbox.refused",
		Disposition: TopicHostOnly,
		Reason:      "why a sandbox could not be applied — the negative half of the confinement inventory",
	},
	{
		Pattern:     "plugin.unsandboxed",
		Disposition: TopicHostOnly,
		Reason:      "announces which sidecars are running with NO filesystem confinement, and why. That is a target list: it names the process to attack and states that nothing will contain it",
	},
	{
		Pattern:     "plugin.install.progress",
		Disposition: TopicHostOnly,
		Reason:      "echoes body.URL VERBATIM from the host-authority POST /plugins/install request (hostOnlyRoute: not even an operator-tier scoped token may call it), with no normalization before the echo. Host-only input must not become view-visible output just because the progress of a long operation is convenient to broadcast",
	},
}

// EventTopicSpec resolves a topic to its classification. ok=false means the
// topic is not in the registry at all, which for a scoped user token is a REFUSAL
// (fail closed) and for a plugin means the manifest `consumes` is the only gate —
// a plugin-defined topic is not host state.
//
// Matching is spelled out here rather than delegated because internal/event
// would be an import cycle away, and the rule is two lines: exact match, or a
// trailing '*' matching a prefix.
func EventTopicSpec(topic string) (spec EventTopic, ok bool) {
	// Exact wins over prefix, so a specific row can be added under a wildcard
	// family later without the wildcard swallowing it.
	for _, t := range eventTopics {
		if t.Pattern == topic {
			return t, true
		}
	}
	for _, t := range eventTopics {
		if n := len(t.Pattern); n > 0 && t.Pattern[n-1] == '*' && strings.HasPrefix(topic, t.Pattern[:n-1]) {
			return t, true
		}
	}
	return EventTopic{}, false
}

// EventTopicCapability returns the capability method a topic's payload is the
// output of, and whether the topic is guarded BY A CAPABILITY. A host-only topic
// reports guarded=false here and is refused by a different arm — callers that
// only ask this question must not conclude "open".
func EventTopicCapability(topic string) (method string, guarded bool) {
	spec, ok := EventTopicSpec(topic)
	if !ok || spec.Disposition != TopicGuardedBy {
		return "", false
	}
	return spec.Method, true
}

// EventTopicHostOnly reports whether a topic is trusted-connections-only.
func EventTopicHostOnly(topic string) bool {
	spec, ok := EventTopicSpec(topic)
	return ok && spec.Disposition == TopicHostOnly
}

// EventTopicIsHostOwned reports whether this topic is published by the host and
// must therefore never be accepted FROM a non-trusted connection. Every
// classified topic is host-published by construction — the registry is derived
// from this repo's publish sites — so classification and ownership are the same
// bit, deliberately: it is not possible to record a topic as host state and
// forget to say who may write it.
func EventTopicIsHostOwned(topic string) bool {
	_, ok := EventTopicSpec(topic)
	return ok
}

// EventTopics returns the registry, ordered by pattern so a failing test prints
// the same thing twice.
func EventTopics() []EventTopic {
	out := append([]EventTopic(nil), eventTopics...)
	sort.Slice(out, func(i, j int) bool { return out[i].Pattern < out[j].Pattern })
	return out
}

// EventTopicGuards returns the guarded subset as topic→method, the shape the
// original two-row table had, for tests and callers that only care about the
// capability-mirroring rows.
func EventTopicGuards() map[string]string {
	out := map[string]string{}
	for _, t := range eventTopics {
		if t.Disposition == TopicGuardedBy {
			out[t.Pattern] = t.Method
		}
	}
	return out
}
