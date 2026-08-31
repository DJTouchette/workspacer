package capspec

import "sort"

// THE HTTP PLANE.
//
// Rounds 6 and 7 closed two planes with the same move. The CALL plane: every
// capability classified, a new method failing until it is. The EVENT plane:
// every published topic classified, a new topic failing until it is. Both
// default CLOSED and both are held to the code by a scan of the registration
// sites rather than to a table that validates itself.
//
// The HTTP plane had none of that. 27 AddRoute sites on the hub, 12 of them
// wrapped in guard(), and nothing anywhere asked what the other 15 serve. What
// they serve, it turned out, included the exact bytes the event plane had just
// classified host-only:
//
//   - GET /plugins encoded mgr.List() verbatim — server.command + args, install
//     argv, source URL, every declared filesystem path scope — to a caller with
//     no credential, over the tailnet, with any Host header. Its event twin,
//     plugin.loaded, is TopicHostOnly: refused to every scoped tier and every
//     plugin. eventtopics.go says so in prose ("GET /plugins currently serves
//     the same thing without a guard(); that route is a separate bug"), and
//     that concession is the whole point — a plane can be closed rigorously and
//     still leak, if the OTHER plane over the same bytes was never asked.
//   - GET /plugins/ui/<id>/ inlined window.__WKS_SETTINGS__ — the merged
//     non-secret setting values — into an unauthenticated HTML document, while
//     the identical read (/plugins/settings) is guard()ed and the identical
//     broadcast (plugin.settings.changed) is TopicHostOnly for exactly that
//     content. The same route served arbitrary host files, because the manifest
//     `ui` field was never validated for "..".
//
// So: every route is classified here, a route the scanner finds and this
// registry does not name FAILS (TestEveryHTTPRouteIsClassified), and a route
// whose bytes have a bus twin must AGREE with that twin's disposition
// (TestUnguardedRoutesAgreeWithTheirBusTwin). The registry cannot lie about the
// code either — the scanner reads whether the site is wrapped in guard() and
// compares.
//
// FOUR SERVERS are covered, because "the HTTP plane" is not one process:
// the hub (:7895), claudemon's API (:7891) and hook (:7890) routers, and the
// MCP facade (:7897). Two of them are Rust. The event plane already scans
// TypeScript from this package for the same reason: a boundary that stops at a
// language boundary is not a boundary.

// RouteDisposition is the closed vocabulary of answers to "who may reach this
// route, and what do they get".
type RouteDisposition string

const (
	// RouteGuarded — the whole route requires a credential. On the hub that is
	// guard() / srv.Authorized: the host token or an operator-scoped token.
	RouteGuarded RouteDisposition = "guarded"

	// RouteHostOnly — guarded, and then some: the route requires the HOST's own
	// credential, and an operator-tier SCOPED token is refused with 403 even
	// though it passes srv.Authorized. Reserved for the acts that make this
	// machine run code (the plugin install family), because the operator tier is
	// what a remote worker node carries — see cmd/hub/hostonly.go. A row of this
	// kind is a STRONGER claim than RouteGuarded, not a different one, so the
	// completeness guard checks it in both directions: the site must apply
	// hostOnly(), and a site that applies hostOnly() may not be recorded as
	// merely guarded.
	RouteHostOnly RouteDisposition = "host-only"

	// RouteTieredPayload — anyone may call it, but WHAT comes back depends on
	// the caller's credential. The disposition exists because guarding some of
	// these routes outright would break a client that legitimately cannot carry
	// a token (a plugin webview, an installed PWA at its start_url), while
	// serving the full payload to everyone is the defect this plane had. A row
	// of this kind MUST name the gate that splits the payload.
	RouteTieredPayload RouteDisposition = "tiered-payload"

	// RoutePublic — the same bytes for every caller, and those bytes carry no
	// host state: embedded assets, vendored library code, an app shell. The
	// reason must say why the bytes are stateless, not why the route is
	// convenient.
	RoutePublic RouteDisposition = "public-by-decision"

	// RouteLoopbackConfined — no credential at any tier; the boundary is the
	// operating system. Every claudemon route is one of these: the listener is
	// pinned to 127.0.0.1, a foreign Host is refused (host_guard), and a
	// cross-site Origin is refused (origin_guard). The disposition is honest
	// rather than flattering — several of these routes grant MORE than any bus
	// tier does, and each such row has to say so and name what confines it.
	RouteLoopbackConfined RouteDisposition = "loopback-confined"
)

// TwinKind says what sort of bus object a route's payload is a twin of.
type TwinKind string

const (
	TwinNone   TwinKind = ""
	TwinEvent  TwinKind = "event-topic"
	TwinMethod TwinKind = "bus-method"
)

// HTTPRoute is one classified route.
type HTTPRoute struct {
	// Server is which listener registers it: "hub", "claudemon-api",
	// "claudemon-hook", "mcp".
	Server string
	// Pattern is the route as REGISTERED (Go mux pattern / axum path), so the
	// scanner can match it literally.
	Pattern     string
	Disposition RouteDisposition
	Reason      string

	// Twin names the bus object carrying the same payload, and TwinKind says
	// which registry to resolve it in. A twin is not decoration: when it is an
	// event topic classified TopicHostOnly, an unguarded route serving the same
	// bytes is a contradiction this package refuses.
	Twin     string
	TwinKind TwinKind

	// Gate names the mechanism that splits a RouteTieredPayload's response
	// (a Go identifier, checked to exist in the hub source).
	Gate string
}

// httpRoutes is the registry. Held to the code from both ends by
// TestEveryHTTPRouteIsClassified.
var httpRoutes = []HTTPRoute{
	// ---- the hub: the bus itself ----------------------------------------
	{
		Server: "hub", Pattern: "/bus", Disposition: RouteGuarded,
		Reason:   "the bus handshake. Classifies the presented token (per-plugin token → that plugin's caps, host/empty token → trusted, scoped token → its tier's method allowlist, else 401) and then runs mayCall/mayPublish/mayConsume per frame. Every other classification in this package is enforced through this route",
		Twin:     "*",
		TwinKind: TwinMethod,
	},
	{
		Server: "hub", Pattern: "/health", Disposition: RouteTieredPayload,
		Reason: "liveness for anyone; subscriber and method COUNTS only for an authorized caller. Topology is host state — how many clients and how many capabilities exist — and an unauthenticated probe gets {status:ok} and nothing else",
		Gate:   "Authorized",
	},

	// ---- the hub: plugin control plane ----------------------------------
	{
		Server: "hub", Pattern: "/plugins", Disposition: RouteTieredPayload,
		Reason:   "THE FINDING. The full Manifest for the trusted host; plugin.PublicManifest — id, name, version and the pane/widget/hotkey contributions — for everyone else. A UI has to know what a plugin contributes to render it and a webview cannot carry the host token, so the split is on the BYTES rather than on the route, and the public half is built by naming what it includes so a new Manifest field is withheld by default",
		Twin:     "plugin.loaded",
		TwinKind: TwinEvent,
		Gate:     "manifestListHandler",
	},
	{
		Server: "hub", Pattern: "/plugins/examples", Disposition: RouteTieredPayload,
		Reason:   "the bundled example catalog, same handler and same split as /plugins. The manifests ship inside the app rather than describing this host, which makes the disclosure weaker — and 'weaker' is not one of the answers this plane has, so it gets the same one",
		Twin:     "plugin.loaded",
		TwinKind: TwinEvent,
		Gate:     "manifestListHandler",
	},
	{
		Server: "hub", Pattern: "/plugins/ui/", Disposition: RouteTieredPayload,
		Reason:   "a webview-only plugin's static assets. The FILES are public (they are the plugin's own front-end, and a <script>/webview URL cannot carry a token), but the injected window.__WKS_SETTINGS__ is not: it is the merged non-secret setting values, which the guard()ed /plugins/settings refuses and which plugin.settings.changed is host-only for. AuthorizedForPlugin gates that block on the host token or the plugin's own bus/pane token. The served ROOT is confined by ValidateUIDir — the `ui` field was unchecked, and `..` or `.` moved that root onto the host filesystem or onto the plugin's own .bus-token",
		Twin:     "plugin.settings.changed",
		TwinKind: TwinEvent,
		Gate:     "AuthorizedForPlugin",
	},
	{
		Server: "hub", Pattern: "/plugins/tokens", Disposition: RouteGuarded,
		Reason: "the per-plugin bus bearer tokens. Reading one IS becoming that plugin on the bus",
	},
	{
		Server: "hub", Pattern: "/plugins/pane-token", Disposition: RouteGuarded,
		Reason: "mints an ephemeral capability-scoped token with ${agentCwd} bound to a caller-supplied cwd — the only way a plugin gets dynamic filesystem roots",
	},
	{
		Server: "hub", Pattern: "/plugins/pane-token/revoke", Disposition: RouteGuarded,
		Reason: "revokes a pane token; operator surface for the same reason minting is",
	},
	{
		Server: "hub", Pattern: "/plugins/settings", Disposition: RouteGuarded,
		Reason:   "reads and writes a plugin's setting values (secrets redacted to __WKS_SECRET__ on the way out). The stated reason plugin.settings.changed is host-only is literally that this route is guarded, so the two must not drift",
		Twin:     "plugin.settings.changed",
		TwinKind: TwinEvent,
	},
	{
		Server: "hub", Pattern: "/plugins/inspect", Disposition: RouteGuarded,
		Reason: "makes the hub fetch a caller-supplied URL from its own network position and parse the manifest — SSRF-shaped, runs no code",
	},
	{
		Server: "hub", Pattern: "/plugins/updates", Disposition: RouteGuarded,
		Reason: "re-fetches every installed plugin's manifest from its recorded Source — the same outbound fetch as inspect, once per plugin",
	},
	{
		Server: "hub", Pattern: "/plugins/install", Disposition: RouteHostOnly,
		Reason:   "download, extract, run the manifest's install argv under explicit consent, re-baseline the grant pin, start the sidecar. The install argv runs on the HUB's own host, so the token guard alone is not the gate: an operator-tier scoped token passes Authorized and every remote worker node carries one, which made a node's bearer string arbitrary code execution here. plugin.install.progress is host-only precisely because it echoes this route's input",
		Twin:     "plugin.install.progress",
		TwinKind: TwinEvent,
	},
	{
		Server: "hub", Pattern: "/plugins/reload", Disposition: RouteHostOnly,
		Reason: "re-reads plugin.json from a CALLER-NAMED directory, re-baselines the consented authority (so a reload can widen a plugin's grants) and starts that directory's sidecar — install-equivalent code execution on this host, from a path the caller chose",
	},
	{
		Server: "hub", Pattern: "/plugins/remove", Disposition: RouteGuarded,
		Reason: "stops the sidecar and deletes the plugin directory",
	},
	{
		Server: "hub", Pattern: "/plugins/setEnabled", Disposition: RouteGuarded,
		Reason: "starts or stops a sidecar and withholds or restores its contributions",
	},
	{
		Server: "hub", Pattern: "/plugins/examples/install", Disposition: RouteHostOnly,
		Reason: "copies a bundled example into the writable plugins dir and runs its install step — install with no network, and gated like install: the step runs on this host, so a scoped operator token (a remote node's) is refused",
	},

	// ---- the hub: clients and static assets ------------------------------
	{
		Server: "hub", Pattern: "/remote", Disposition: RouteGuarded,
		Reason: "the remote web client's entry document. HTML only, no token baked in — guarded because it is the remote entrypoint",
	},
	{
		Server: "hub", Pattern: "/app/", Disposition: RouteTieredPayload,
		Reason: "the full React renderer. The ENTRY DOCUMENT requires the token; the hashed asset bundle is served unauthenticated and long-cached. The split is inside one handler because http.FileServer serves a tree, and the renderer's data all arrives over /bus, which is where the boundary is",
		Gate:   "Authorized",
	},
	{
		Server: "hub", Pattern: "/m", Disposition: RoutePublic,
		Reason: "the mobile PWA shell. Deliberately unguarded: an installed PWA launches at start_url with no token available, so the shell must load and then gate on its own stored token. The document carries no host state",
	},
	{
		Server: "hub", Pattern: "/plugins/origin", Disposition: RoutePublic,
		Reason: "the second origin the OPERATOR declared for this hub (--plugin-origin), which browser clients must read before they can frame a plugin cross-origin — necessarily before they have shown anything. It is a URL the operator chose to publish (it is about to appear in every plugin iframe's src), not host state: no plugin is named, no port is disclosed, and it answers {\"origin\":\"\"} when undeclared",
	},
	{
		Server: "hub", Pattern: "/plugins/sdk.js", Disposition: RoutePublic,
		Reason: "the host-owned plugin SDK (window.workspacer): static library code with no host state, injected into every plugin webview by a <script> tag that cannot carry a token",
	},
	{
		Server: "hub", Pattern: "/manifest.webmanifest", Disposition: RoutePublic,
		Reason: "static PWA manifest bytes; the browser fetches it without our credential and it describes only the app's own icons and start_url",
	},
	{
		Server: "hub", Pattern: "/sw.js", Disposition: RoutePublic,
		Reason: "static service-worker JS for the PWA; embedded at build time, identical on every host",
	},
	{
		Server: "hub", Pattern: "/icon-192.png", Disposition: RoutePublic,
		Reason: "embedded PNG bytes of the app icon — the same file in every build, no host state",
	},
	{
		Server: "hub", Pattern: "/icon-512.png", Disposition: RoutePublic,
		Reason: "embedded PNG bytes of the app icon — the same file in every build, no host state",
	},
	{
		Server: "hub", Pattern: "/icon-maskable-512.png", Disposition: RoutePublic,
		Reason: "embedded PNG bytes of the maskable app icon — the same file in every build, no host state",
	},
	{
		Server: "hub", Pattern: "/apple-touch-icon.png", Disposition: RoutePublic,
		Reason: "embedded PNG bytes of the iOS home-screen icon — the same file in every build, no host state",
	},
	{
		Server: "hub", Pattern: "/xterm.js", Disposition: RoutePublic,
		Reason: "vendored xterm library bytes for the remote terminal mirror; a <script> tag cannot carry the bus token and the file is content-pinned to a vendored version",
	},
	{
		Server: "hub", Pattern: "/xterm.css", Disposition: RoutePublic,
		Reason: "vendored xterm stylesheet; a <link> tag cannot carry the bus token and the file is content-pinned to a vendored version",
	},
	{
		Server: "hub", Pattern: "/addon-fit.js", Disposition: RoutePublic,
		Reason: "vendored xterm fit-addon bytes, same audience and same reasoning as /xterm.js",
	},

	// ---- the MCP facade (:7897) -----------------------------------------
	{
		Server: "mcp", Pattern: "/mcp", Disposition: RouteGuarded,
		Reason:   "the MCP streamable-HTTP bridge onto the bus. requireScope resolves the credential to a tool TIER: the static -mcp-token (constant-time compare) is operator, a tokens.json scoped token gets its own tier's server, a present-but-unknown token is 401, and no-credential-at-all is operator only when no static token is set — the loopback-open default checkBindPolicy permits solely for a loopback bind",
		Twin:     "*",
		TwinKind: TwinMethod,
	},
	{
		Server: "mcp", Pattern: "/sse", Disposition: RouteGuarded,
		Reason:   "the SSE half of the same bridge, behind the same requireScope",
		Twin:     "*",
		TwinKind: TwinMethod,
	},
	{
		Server: "mcp", Pattern: "/health", Disposition: RoutePublic,
		Reason: "{status, hubConnected} for liveness probes. requireHost wraps it too, so it is reachable only from loopback or the configured bind address; the payload is one boolean about our own uplink",
	},

	// ---- claudemon: the API router (:7891) -------------------------------
	// Every row here is loopback-confined by the same three mechanisms
	// (127.0.0.1 bind, host_guard, origin_guard) and each names what it grants
	// RELATIVE TO THE BUS, because several grant strictly more than any bus tier
	// and that asymmetry is the thing worth writing down.
	{
		Server: "claudemon-api", Pattern: "/sessions", Disposition: RouteLoopbackConfined,
		Reason:   "the full fleet listing. agents.list / sessions.snapshots are VIEW-tier on the bus, so this route grants no more than a scoped token already has — the confinement is the only difference, not the disposition",
		Twin:     "agents.list",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/spawn", Disposition: RouteLoopbackConfined,
		Reason:   "takes caller-supplied argv + cwd + env and pty::spawn()s it as the desktop user. Its bus twin agents.spawn NEVER lets the caller supply argv: the brain builds it from a vetted profile, forces skipPermissions=false, drops an escalating permissionMode and scrubs the profile's bypass extraArgs. This route is therefore strictly more powerful than the bus one, and nothing but the loopback boundary stands between them",
		Twin:     "agents.spawn",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/spawn-managed", Disposition: RouteLoopbackConfined,
		Reason:   "honours yolo:true and permission_mode:\"bypassPermissions\" verbatim. The brain's comment on the bus path asserts the caller has already clamped off every bypass — that clamp lives on the bus side only, so the same asymmetry as /sessions/spawn, one bypass flag closer",
		Twin:     "agents.spawn",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/providers/:provider/models", Disposition: RouteLoopbackConfined,
		Reason:   "forks the resolved provider CLI (`opencode models`) in a caller-supplied cwd. The caller-supplied ?bin= is ignored — an earlier fix, still pinned — but the cwd is honoured, and as a simple GET this was reachable cross-site until origin_guard: no preflight fires, so CORS withheld the response while the process ran anyway",
		Twin:     "agents.spawn",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id", Disposition: RouteLoopbackConfined,
		Reason:   "one session's snapshot — sessions.snapshot is VIEW-tier on the bus, so this discloses nothing a scoped token is refused",
		Twin:     "sessions.snapshot",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/input", Disposition: RouteLoopbackConfined,
		Reason:   "writes raw keystrokes into a live PTY. On the bus this rides sessions.attachTerminal / terminals.*, which are in neither scoped tier; the pty.bytes.* topic mirroring its output is TopicGuardedBy that same method",
		Twin:     "sessions.attachTerminal",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/message", Disposition: RouteLoopbackConfined,
		Reason:   "the settle+verify flush send path — injects a prompt into a running agent. agents.sendMessage is TRIAGE-tier, so a scoped credential can already do this over the bus",
		Twin:     "agents.sendMessage",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/approve", Disposition: RouteLoopbackConfined,
		Reason:   "resolves a parked permission prompt. claude.approve is TRIAGE-tier and is half of the one accepted composition pair, so the bus grants this to a scoped token too",
		Twin:     "claude.approve",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/answer", Disposition: RouteLoopbackConfined,
		Reason:   "answers an AskUserQuestion picker. claude.answer is TRIAGE-tier; its params are classified as the same PTY bytes sessions.terminalInput carries",
		Twin:     "claude.answer",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/decide", Disposition: RouteLoopbackConfined,
		Reason:   "records a decision on a parked prompt — the same approval surface as /approve, arriving through the decision-record path rather than the yes/no one",
		Twin:     "claude.approve",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/gate", Disposition: RouteLoopbackConfined,
		Reason:   "arms or disarms the approval gate. claude.gate is deliberately OPERATOR-only and its exclusion from triage is pinned by composition_test — so this route hands a loopback caller a verb the bus refuses every scoped tier, and it is the sharpest asymmetry on this router after spawn",
		Twin:     "claude.gate",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/signal", Disposition: RouteLoopbackConfined,
		Reason:   "sends SIGINT / interrupt to the child. claude.signal is TRIAGE-tier on the bus",
		Twin:     "claude.signal",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/permission-mode", Disposition: RouteLoopbackConfined,
		Reason:   "live permission-mode switch. The bus twin runs the shared escalation allow-list (assertNoPermissionBypass, the 'second door' the spawn clamp names); this route applies only its own vocabulary validation, so escalation is refused on the bus and accepted here",
		Twin:     "claude.setPermissionMode",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/model", Disposition: RouteLoopbackConfined,
		Reason:   "owner boundary for live model switching: managed threads receive a structural update, while Claude PTY gets a daemon-built command only after model validation. PTY effort is refused as unapplied, and the response distinguishes queued delivery from accepted delivery. Model choice remains operator business on the bus and owner-authored requested_selection remains canonical",
		Twin:     "claude.setModel",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/resize", Disposition: RouteLoopbackConfined,
		Reason:   "resizes the session PTY — terminal geometry, operator-tier on the bus (terminals.*), harmless beyond the terminal it resizes",
		Twin:     "sessions.attachTerminal",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/output", Disposition: RouteLoopbackConfined,
		Reason:   "the raw PTY scrollback for one session. On the bus the equivalent stream is guarded by sessions.attachTerminal, which is in NEITHER scoped tier — so this is more available on HTTP than on the bus, bounded only by loopback",
		Twin:     "sessions.attachTerminal",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/stream", Disposition: RouteLoopbackConfined,
		Reason:   "the live SSE feed of the same raw PTY bytes, with the same relationship to sessions.attachTerminal and the pty.bytes.* topic it guards",
		Twin:     "sessions.attachTerminal",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/transcript", Disposition: RouteLoopbackConfined,
		Reason:   "the on-disk transcript, with the session id validated against traversal by valid_session_id. sessions.transcript is VIEW-tier on the bus",
		Twin:     "sessions.transcript",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/conversation", Disposition: RouteLoopbackConfined,
		Reason:   "the parsed conversation with ?since= deltas. sessions.conversation is VIEW-tier on the bus",
		Twin:     "sessions.conversation",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/subagents/:agent_id/conversation", Disposition: RouteLoopbackConfined,
		Reason:   "the parsed provider-owned child-thread conversation. sessions.subagentConversation is VIEW-tier on the bus and claudemon validates the child id belongs to the parent session",
		Twin:     "sessions.subagentConversation",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/sessions/:id/handoff", Disposition: RouteLoopbackConfined,
		Reason:   "builds a cross-provider handoff brief and PERSISTS it under ~/.workspacer/handoffs/. No bus capability writes handoff files; the nearest read (sessions.conversation) is view-tier and read-only, so the write half exists only here",
		Twin:     "sessions.conversation",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/conversation/stream", Disposition: RouteLoopbackConfined,
		Reason:   "global conversation deltas as SSE — the streaming form of the view-tier sessions.conversation read",
		Twin:     "sessions.conversation",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/events", Disposition: RouteLoopbackConfined,
		Reason:   "the daemon event feed the hub bridges onto the bus. Downstream it becomes agent.state_changed / agent.snapshot, both classified open-by-decision, so the feed itself discloses what the view tier already receives",
		Twin:     "agent.snapshot",
		TwinKind: TwinEvent,
	},
	{
		Server: "claudemon-api", Pattern: "/hooks/stream", Disposition: RouteLoopbackConfined,
		Reason:   "raw hook events as SSE — the pre-mapping form of the same feed, carrying the tool inputs a hook reports before the state machine folds them into a snapshot",
		Twin:     "agent.state_changed",
		TwinKind: TwinEvent,
	},
	{
		Server: "claudemon-api", Pattern: "/statusline/stream", Disposition: RouteLoopbackConfined,
		Reason:   "statusline frames as SSE. agent.statusline is TopicGuardedBy sessions.snapshot on the bus AND filtered by the fleet-visibility rule before publication; this route applies no visibility filter, which is a second asymmetry the loopback boundary is carrying",
		Twin:     "agent.statusline",
		TwinKind: TwinEvent,
	},
	{
		Server: "claudemon-api", Pattern: "/usage", Disposition: RouteLoopbackConfined,
		Reason:   "account-level rate-limit windows fetched with the user's OAuth credentials. NO bus capability exposes account usage at any tier; the hub's keep-warm path documents this route as deliberately ungated, which makes loopback the whole of its protection",
		Twin:     "",
		TwinKind: TwinNone,
	},
	{
		Server: "claudemon-api", Pattern: "/usage/report", Disposition: RouteLoopbackConfined,
		Reason:   "the WHOLE usage picture, and the widest DISCLOSURE on this router: every provider and every configured login in one document — the account key (which IS the profile's absolute configDir path), its label and default flag, 5h/7d/monthly window percentages with reset times, cumulative spend in USD and nano-AIU, the input/cache-read/cache-write/output/reasoning token split, a per-model breakdown, per-account live-session counts, and a failure object that can carry needs_reauth. It answers from the store and from what the provider CLIs left on disk, so a just-booted daemon with no live session serves all of it. THE OTHER TWO PLANES AGREE ABOUT THESE BYTES, and both withhold them from every scoped tier: no bus capability returns account-level usage at any tier (the /usage row's sentence still holds — the usage-at-boot work added no method and no topic), the nearest account-IDENTITY read claude.profiles.list hands back the same configDir strings and is in NEITHER viewMethods nor triageMethods, and the nearest event twin agent.statusline — five_hour_pct and cost_usd for ONE session — is TopicGuardedBy sessions.snapshot and fleet-visibility filtered before publication. So this route grants strictly more than any bus tier and strictly more than its own sibling /usage (the default login's three percentages), and 127.0.0.1 + host_guard + origin_guard are the whole of what stands between an unauthenticated caller and every login's identity, spend and quota headroom on this machine",
		Twin:     "",
		TwinKind: TwinNone,
	},
	{
		Server: "claudemon-api", Pattern: "/heartbeat", Disposition: RouteLoopbackConfined,
		Reason: "records a wrapper heartbeat: liveness bookkeeping for a process that already holds the session's PTY",
	},
	{
		Server: "claudemon-api", Pattern: "/heartbeats", Disposition: RouteLoopbackConfined,
		Reason: "lists wrapper heartbeats — session ids and timestamps, a strict subset of the view-tier fleet listing",
	},
	{
		Server: "claudemon-api", Pattern: "/oneshot", Disposition: RouteLoopbackConfined,
		Reason:   "runs a one-shot agent invocation. Spawn-class with no scoped-tier equivalent at all, so it sits with /sessions/spawn behind the same loopback boundary",
		Twin:     "agents.spawn",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/wrapper/:id", Disposition: RouteLoopbackConfined,
		Reason:   "the wrapper WebSocket: registers and drives a session's PTY. CORS never applies to an upgrade, so this route calls origin_allowed by hand BEFORE upgrading — the same predicate the router-wide origin_guard now applies to everything else",
		Twin:     "sessions.attachTerminal",
		TwinKind: TwinMethod,
	},
	{
		Server: "claudemon-api", Pattern: "/mcp/ask/:session_id", Disposition: RouteLoopbackConfined,
		Reason: "the MCP streamable-HTTP endpoint that gives an agent an AskUserQuestion tool, parking a structured question in the GUI. POST-only; it creates a prompt for a human rather than answering one",
	},
	{
		Server: "claudemon-api", Pattern: "/health", Disposition: RouteLoopbackConfined,
		Reason: "the literal string \"ok\". No host state at all; it is loopback-confined only because it shares the router's layers",
	},

	// ---- claudemon: the hook ingress router (:7890) ----------------------
	{
		Server: "claudemon-hook", Pattern: "/hook", Disposition: RouteLoopbackConfined,
		Reason:   "ingests Claude Code hook events straight into the session state machine, the broadcast fanout and SQLite. On the bus, mayPublish refuses a non-trusted connection the agent.* topics this becomes (EventTopicIsHostOwned) — precisely because internal/push treats agent.snapshot as authoritative; over HTTP any loopback process forges them for free, and host_guard + origin_guard are what keep a browser out of that set",
		Twin:     "agent.state_changed",
		TwinKind: TwinEvent,
	},
	{
		Server: "claudemon-hook", Pattern: "/hook/:kind", Disposition: RouteLoopbackConfined,
		Reason:   "the same ingress with the event kind taken from the URL segment, overriding the body's own `event` field. Same forging reach as /hook",
		Twin:     "agent.state_changed",
		TwinKind: TwinEvent,
	},
	{
		Server: "claudemon-hook", Pattern: "/statusline", Disposition: RouteLoopbackConfined,
		Reason:   "ingests statusline frames — model, cost, context and rate-limit percentages — into the same state machine. agent.statusline is capability-guarded on the bus; this is the write side of it",
		Twin:     "agent.statusline",
		TwinKind: TwinEvent,
	},
	{
		Server: "claudemon-hook", Pattern: "/health", Disposition: RouteLoopbackConfined,
		Reason: "the literal string \"ok\" on the ingress port. No host state; confined because it shares the router's layers",
	},
}

// HTTPRouteSpec resolves one route to its classification.
func HTTPRouteSpec(server, pattern string) (HTTPRoute, bool) {
	for _, r := range httpRoutes {
		if r.Server == server && r.Pattern == pattern {
			return r, true
		}
	}
	return HTTPRoute{}, false
}

// HTTPRoutes returns the registry ordered by (server, pattern), so a failing
// test prints the same thing twice.
func HTTPRoutes() []HTTPRoute {
	out := append([]HTTPRoute(nil), httpRoutes...)
	sort.Slice(out, func(i, j int) bool {
		if out[i].Server != out[j].Server {
			return out[i].Server < out[j].Server
		}
		return out[i].Pattern < out[j].Pattern
	})
	return out
}

// HTTPRouteServesHostOnlyPayload reports whether a route's bytes have an event
// twin this package classifies TopicHostOnly — i.e. bytes the bus refuses every
// scoped tier and every plugin. Such a route may not be reachable without a
// credential unless its payload is split by a named gate; that is
// TestUnguardedRoutesAgreeWithTheirBusTwin.
func HTTPRouteServesHostOnlyPayload(r HTTPRoute) bool {
	return r.TwinKind == TwinEvent && EventTopicHostOnly(r.Twin)
}
