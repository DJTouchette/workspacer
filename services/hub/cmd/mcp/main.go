// Command mcp is the workspacer MCP facade.
//
// It exposes the hub's capabilities — list / spawn / drive Claude agents and
// terminals — as MCP tools over HTTP, so Claude Code (or any MCP client) can
// drive workspacer headlessly via `--mcp-config`.
//
// It is a thin adapter: each tool call is forwarded to the hub bus as a
// capability `call`, and the provider (the Electron main process) executes it.
// The facade never touches workspacer state directly — it routes, exactly like
// the hub does. That keeps the substrate generic and the facade replaceable.
//
// Two HTTP transports are served from the same MCP server:
//
//	/mcp  — Streamable HTTP (the current MCP HTTP transport; uses SSE to stream)
//	/sse  — legacy SSE transport, for older clients
package main

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"slices"
	"strings"
	"syscall"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/parentwatch"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func main() {
	addr := flag.String("addr", "127.0.0.1:7897", "HTTP listen address for the MCP server")
	hubURL := flag.String("hub", "ws://127.0.0.1:7895/bus", "workspacer hub bus WebSocket URL")
	token := flag.String("token", os.Getenv("HUB_TOKEN"), "hub bus token (when the hub requires auth)")
	// mcpToken guards the facade's OWN inbound HTTP surface (/mcp, /sse) — distinct
	// from -token, which authenticates the facade's OUTBOUND connection to the hub
	// bus. Env mirrors the hub's HUB_TOKEN convention (flag/env, no token file).
	mcpToken := flag.String("mcp-token", os.Getenv("WKS_MCP_TOKEN"), "bearer token required on /mcp and /sse (empty = no auth; REQUIRED to bind a non-loopback -addr)")
	// tokensPath is the same tokens.json the hub bus reads for its scoped
	// capability tokens. The desktop mints a per-session record here at spawn
	// (label `session:<id>`) so a spawned agent can present a bearer that
	// resolves to a TIER of the facade's tools — view for summarizer workers,
	// triage for attention-handlers, operator for supervisors — instead of the
	// whole surface. The store is mtime-gated, so mint/revoke take effect on the
	// next request without restarting the facade.
	tokensPath := flag.String("tokens", authtoken.DefaultPath(), "scoped capability-token file (tokens.json) for per-session facade tiers")
	// untokened is the deliberate dial on the credential-less loopback default:
	// operator keeps the historical open-on-loopback behavior, view serves the
	// read-only tier to bare requests, deny 401s them (every request must then
	// present the static token or a tokens.json scoped token).
	untokened := flag.String("untokened", envOr("WKS_MCP_UNTOKENED", untokenedOperator), "access tier for credential-less requests: operator (back-compat default), view (read-only tier), or deny (401)")
	flag.Parse()

	if err := checkUntokenedMode(*untokened); err != nil {
		log.Fatalf("mcp: %v", err)
	}
	// Fail closed: a non-loopback bind with no token lets anyone who reaches the
	// port drive the whole agent fleet. Loopback-with-no-token stays open (the
	// default the desktop relies on).
	if err := checkBindPolicy(*addr, *mcpToken, *untokened); err != nil {
		log.Fatalf("mcp: %v", err)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	// Self-exit if the launcher (the desktop app) dies, so we don't orphan and
	// keep port 7897 alive. No-op when run manually.
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	parentwatch.Watch(cancel)

	client := busclient.New(*hubURL, *token)
	go client.Run(ctx)

	gate := &authGate{static: *mcpToken, store: authtoken.NewStore(*tokensPath), untokened: *untokened}
	if *mcpToken != "" {
		log.Printf("mcp auth enabled (bearer token required on /mcp and /sse)")
	}
	if *untokened != untokenedOperator {
		log.Printf("mcp untokened access dialed to %q", *untokened)
	}
	// Plugin-contributed tools: poll the hub's consented surface and graft it
	// onto per-token servers (opt-in via each session token's plugin grants).
	catalog := newPluginCatalog(client)
	go catalog.run(ctx)
	mux := newMux(newServerCache(client, catalog, tierServers(client)), client, gate)

	httpSrv := &http.Server{Addr: *addr, Handler: servedHandler(*addr, mux)}
	go func() {
		log.Printf("mcp facade listening on %s (streamable: http://%s/mcp, sse: http://%s/sse)", *addr, *addr, *addr)
		log.Printf("bridging to hub %s", *hubURL)
		if err := httpSrv.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			log.Fatalf("mcp: %v", err)
		}
	}()

	<-ctx.Done()
	stop()
	log.Println("mcp facade shutting down")
	shutCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = httpSrv.Shutdown(shutCtx)
}

// newMux builds the facade's HTTP router. /mcp and /sse resolve each request's
// credential to its token record via the authGate and serve that record's
// server — the tier (view/triage/operator), plus any plugin tools the record
// grants; /health stays open (unauthenticated) so liveness probes work without
// a secret.
func newMux(cache *serverCache, client *busclient.Client, gate *authGate) *http.ServeMux {
	getServer := func(r *http.Request) *mcp.Server {
		rec, ok := gate.resolveRecord(r)
		if !ok {
			// requireScope already rejected unresolvable requests; reaching here
			// means the token was revoked between the gate and this lookup. Serve
			// the empty tier rather than any tools.
			return newDeniedServer()
		}
		return cache.serverFor(rec)
	}

	mux := http.NewServeMux()
	mux.Handle("/mcp", requireScope(gate, mcp.NewStreamableHTTPHandler(getServer, nil)))
	mux.Handle("/sse", requireScope(gate, mcp.NewSSEHandler(getServer, nil)))
	mux.HandleFunc("/health", func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(map[string]any{
			"status":       "ok",
			"hubConnected": client.Ready(),
		})
	})
	return mux
}

// servedHandler is what the facade actually serves: the mux with the Host pin
// wrapped around ALL of it. Named (rather than composed inline in main) so a
// test can assert the composition rather than only the predicate — every
// hostguard case builds its own guarded() helper, so the whole DNS-rebinding
// defense could be deleted from main with a green suite. The hub twin pins the
// same thing via bus.Server.Handler().
func servedHandler(bindAddr string, mux http.Handler) http.Handler {
	return requireHost(bindAddr, mux)
}

// requireHost rejects a request whose `Host` header is neither loopback nor the
// concrete address this facade was told to bind.
//
// This is the DNS-rebinding defense, and on the loopback default it is the ONLY
// thing standing between a web page and the fleet. A browser cannot normally
// reach this port cross-origin — a JSON-RPC POST is preflighted and no CORS
// headers come back — but rebinding sidesteps that entirely: the attacker's own
// name resolves to 127.0.0.1, so the request is same-origin, no preflight is
// sent, and the credential-free bearer default lets it straight through. What
// rebinding cannot forge is the `Host` header, which still carries the
// attacker's domain.
//
// Deliberately wrapped around the WHOLE mux, /health included. Every legitimate
// client dials 127.0.0.1 and sends a loopback Host; nothing that reaches this
// port by name has business here.
//
// TWIN: claudemon's `host_guard` / `AllowedHosts::permits`
// (services/claudemon/src/daemon/api.rs) — same rule, same shape. A request with
// no Host at all is allowed, matching that twin: HTTP/1.0 and some local probes
// omit it, and it is not a header an attacker gains anything by dropping.
func requireHost(bindAddr string, h http.Handler) http.Handler {
	// A wildcard bind names no host, so it adds nothing to the allowlist.
	extra := hostWithoutPort(bindAddr)
	if extra == "0.0.0.0" || extra == "::" {
		extra = ""
	}
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if host := r.Host; host != "" {
			name := hostWithoutPort(host)
			if !hostIsLoopback(name) && (extra == "" || name != extra) {
				http.Error(w, "host not allowed", http.StatusForbidden)
				return
			}
		}
		h.ServeHTTP(w, r)
	})
}

// hostWithoutPort strips a trailing `:port` from a Host/authority, handling
// bracketed IPv6 (`[::1]:7897` → `::1`).
func hostWithoutPort(h string) string {
	if rest, ok := strings.CutPrefix(h, "["); ok {
		if name, _, found := strings.Cut(rest, "]"); found {
			return name
		}
		return h
	}
	if name, _, found := strings.Cut(h, ":"); found {
		return name
	}
	return h
}

// hostIsLoopback reports whether a bare hostname names this machine's loopback:
// the literal `localhost`, or any address in 127.0.0.0/8 / ::1.
func hostIsLoopback(name string) bool {
	if strings.EqualFold(name, "localhost") {
		return true
	}
	ip := net.ParseIP(name)
	return ip != nil && ip.IsLoopback()
}

// authGate resolves a request's credential to a tool tier.
//
// Two credential kinds, opposite lifetimes:
//   - static (-mcp-token / WKS_MCP_TOKEN): the facade-wide secret. Matching it
//     is operator — it exists to guard non-loopback binds, same as before.
//   - store (tokens.json): per-session scoped tokens the desktop mints at
//     spawn. A match grants that record's TIER (view/triage/operator), which is
//     both the tool list the client sees and the calls it may make.
//
// No credential at all is governed by the untokened dial (-untokened /
// WKS_MCP_UNTOKENED) — operator by default, the loopback-open back-compat every
// existing client (and the desktop's own supervisor spawns) relies on; view
// serves only the read-only tier; deny refuses. The static token, when set,
// still overrides the dial as it always has: setting it means "credentials
// required", so a credential-less request is refused regardless of the dial. A
// credential that is PRESENT but unknown is 401, never open access: presenting
// a revoked session token must not quietly escalate to the untokened default.
type authGate struct {
	static string
	store  *authtoken.Store
	// untokened is one of untokenedOperator/View/Deny. The zero value behaves
	// as operator, so an authGate built without the field keeps the historical
	// default.
	untokened string
}

// The untokened-access dial's positions.
const (
	untokenedOperator = "operator"
	untokenedView     = "view"
	untokenedDeny     = "deny"
)

// checkUntokenedMode refuses an unrecognized dial value at startup — a typo in
// a lockdown flag must not silently fall back to the open default.
func checkUntokenedMode(mode string) error {
	switch mode {
	case untokenedOperator, untokenedView, untokenedDeny:
		return nil
	}
	return fmt.Errorf("invalid -untokened value %q: want operator, view, or deny", mode)
}

// envOr returns the environment variable's value, or def when unset/empty.
func envOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

// presentedToken extracts the request's credential: an `Authorization: Bearer`
// header first, else the `t` query parameter — for clients whose MCP config
// carries only a URL (codex `-c mcp_servers…url`, opencode.json), which cannot
// send headers. A malformed Authorization header is returned verbatim so it
// fails the lookups below (fail closed) instead of falling through to the
// open default.
func presentedToken(r *http.Request) string {
	if h := r.Header.Get("Authorization"); h != "" {
		if tok, ok := strings.CutPrefix(h, "Bearer "); ok {
			return tok
		}
		return h
	}
	return r.URL.Query().Get("t")
}

// resolveRecord maps a request to the token record governing it: the tier it
// may use plus any per-token plugin grants. The static token and the untokened
// loopback default both synthesize a plain operator record — NO plugin grants;
// plugin tools are strictly opt-in per session token. The static compare is
// constant-time to avoid leaking the token via timing; store lookups are
// mtime-gated reads of tokens.json.
func (g *authGate) resolveRecord(r *http.Request) (authtoken.Record, bool) {
	tok := presentedToken(r)
	if tok == "" {
		// A set static token has always meant "credentials required": refuse
		// regardless of the dial (it can only be stricter than the dial's view/
		// operator positions, never looser).
		if g.static != "" {
			return authtoken.Record{}, false
		}
		switch g.untokened {
		case untokenedDeny:
			return authtoken.Record{}, false
		case untokenedView:
			// Read-only tier, and — like every synthesized record — NO plugin
			// grants; plugin tools stay strictly opt-in per session token.
			return authtoken.Record{Scope: authtoken.ScopeView}, true
		default: // operator, or the zero value (back-compat)
			return authtoken.Record{Scope: authtoken.ScopeOperator}, true
		}
	}
	if g.static != "" && subtle.ConstantTimeCompare([]byte(tok), []byte(g.static)) == 1 {
		return authtoken.Record{Scope: authtoken.ScopeOperator}, true
	}
	if g.store != nil {
		if rec, ok := g.store.Lookup(tok); ok {
			return rec, true
		}
	}
	return authtoken.Record{}, false
}

// resolve is resolveRecord reduced to the tier — what requireScope and most
// tests care about.
func (g *authGate) resolve(r *http.Request) (authtoken.Scope, bool) {
	rec, ok := g.resolveRecord(r)
	return rec.Scope, ok
}

// requireScope wraps h so only requests the gate can resolve reach it. The
// tier itself is applied by newMux's getServer; this just turns "no/unknown
// credential" into a 401 before the MCP handler sees the request.
func requireScope(gate *authGate, h http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if _, ok := gate.resolve(r); !ok {
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		h.ServeHTTP(w, r)
	})
}

// checkBindPolicy fails closed when the facade would expose its fleet-driving
// surface beyond the local host without a token. Loopback binds may stay open
// (the desktop default); anything reachable from the network must carry a
// token — OR run with `-untokened deny`, which is strictly stronger than the
// static-token requirement it substitutes for: every request must then present
// a resolvable credential (the static token merely being one way to have one),
// and credential-less requests 401 outright. `view` does NOT satisfy the
// policy: it still serves tools to anyone who reaches the port.
func checkBindPolicy(addr, token, untokened string) error {
	if token == "" && untokened != untokenedDeny && !isLoopbackAddr(addr) {
		return fmt.Errorf("refusing to bind non-loopback address %q without an auth token: set WKS_MCP_TOKEN / -mcp-token, or -untokened deny (anyone reaching this port can drive the whole agent fleet)", addr)
	}
	return nil
}

// isLoopbackAddr reports whether a listen address binds only the loopback
// interface. A bare port (":7897"), 0.0.0.0, or :: all reach the network and
// are treated as non-loopback; an unresolved hostname is likewise treated as
// non-loopback to fail safe.
func isLoopbackAddr(addr string) bool {
	host, _, err := net.SplitHostPort(addr)
	if err != nil {
		host = addr // no port; treat the whole thing as the host
	}
	if host == "" {
		return false // ":7897" binds every interface
	}
	if host == "localhost" {
		return true
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

// tierServers builds one MCP server per capability tier. Which tools a tier
// holds is DERIVED from authtoken's scope allowlists — the same patterns the
// hub bus enforces on scoped connections — so the facade and the bus can never
// disagree about what "view" means, and a method admitted to a tier later
// lights up its tool here with no facade change.
func tierServers(c *busclient.Client) map[authtoken.Scope]*mcp.Server {
	return map[authtoken.Scope]*mcp.Server{
		authtoken.ScopeView:     newServer(c, authtoken.ScopeView),
		authtoken.ScopeTriage:   newServer(c, authtoken.ScopeTriage),
		authtoken.ScopeOperator: newServer(c, authtoken.ScopeOperator),
	}
}

// newDeniedServer is the empty tier: a valid MCP server exposing no tools.
// Served on the (revocation-race / unknown-scope) edges where a request passed
// the gate but no tier can be established.
func newDeniedServer() *mcp.Server {
	return mcp.NewServer(&mcp.Implementation{
		Name:    "workspacer",
		Title:   "Workspacer",
		Version: "0.1.0",
	}, nil)
}

// build accumulates one tier's tools as they register. The `help` tool renders
// its docs from this registry, so what help says can't drift from what the
// tier actually exposes.
type build struct {
	s     *mcp.Server
	c     *busclient.Client
	scope authtoken.Scope
	allow []string // method patterns this tier may call (authtoken Scope.Methods)
	group string   // current section, stamped onto tools as they register
	tools []toolInfo
	// profiles is the token record's profile-dispatch grant
	// (authtoken.Record.ProfilesAllowed): the Claude profile ids spawn_agent may
	// name. Enforced HERE because the facade multiplexes every session token
	// over one trusted bus connection — the hub sees the facade's credential,
	// not the session's, so the per-record check must happen where the record
	// is resolved. Exact ids only; empty = spawn_agent refuses any profileId.
	profiles []string
	// yolo is the token record's full-access grant (authtoken.Record.YoloAllowed):
	// whether spawn_agent may forward the caller's skipPermissions request
	// instead of clamping it off. Enforced HERE for the SAME structural reason
	// as profiles — the hub stamps `yoloGranted` for the facade's ONE trusted
	// host-token connection regardless of which session is multiplexed over it,
	// so the per-session grant can only be honored where the session's own
	// record was resolved. Default false: an ungranted session's spawn is
	// clamped, exactly like a bus caller's.
	yolo bool
}

type toolInfo struct {
	Name, Desc, Method, Group string
}

func (b *build) allowed(method string) bool { return event.MatchesAny(b.allow, method) }

// newServer wires the hub capabilities a tier may call to MCP tools. Names are
// snake_case (the MCP tool convention); descriptions are written for the model
// and kept to ONE line each — the schemas are what every connected agent pays
// context for, so usage guidance lives behind the `help` tool instead.
func newServer(c *busclient.Client, scope authtoken.Scope) *mcp.Server {
	return newServerWithGrants(c, scope, nil, nil, false)
}

// newServerWithGrants additionally applies a token record's per-token grants:
// plugin-contributed tools grafted onto the tier (see plugins.go), the
// profile-dispatch allowlist spawn_agent checks a profileId against, and the
// full-access grant (yolo) that lets spawn_agent forward a skipPermissions
// request instead of clamping it. Used by the serverCache for tokens whose
// record grants any of these.
func newServerWithGrants(c *busclient.Client, scope authtoken.Scope, plugins []grantedPluginTools, profiles []string, yolo bool) *mcp.Server {
	s := mcp.NewServer(&mcp.Implementation{
		Name:    "workspacer",
		Title:   "Workspacer",
		Version: "0.1.0",
	}, nil)
	b := &build{s: s, c: c, scope: scope, allow: scope.Methods(), profiles: profiles, yolo: yolo}

	// ── Observe ────────────────────────────────────────────────────────────
	b.group = "observe"
	addFleetTool[listAgentsIn](b, "list_agents",
		"List running Claude Code agent sessions with their state, model, context usage, and any pending approval or question. The lightweight fleet overview, spanning federated peer hubs (remote rows carry a hub field); use get_snapshot for full detail on one session.",
		"agents.list")
	addHubTool[transcriptIn](b, "get_transcript",
		"Fetch a session's transcript so you can see the context behind a pending approval or question before acting.",
		"sessions.transcript")
	addHubTool[conversationIn](b, "get_conversation",
		"Fetch a session's parsed conversation items plus the latest sequence number; pass sinceSeq to get only items after that sequence (cheap incremental polling).",
		"sessions.conversation")
	addHubTool[sessionIn](b, "get_snapshot",
		"Get the full live snapshot for one session: conversation turns, tool calls, usage/cost, subagents, workflow runs, and any pending approval/question. Heavier than list_agents — use it to inspect a single agent in depth.",
		"sessions.snapshot")
	addFleetTool[listAgentsIn](b, "list_snapshots",
		"Get full snapshots for every session at once, spanning federated peer hubs (verbose — large payload; remote rows carry a hub field). Prefer list_agents for an overview and get_snapshot for one session.",
		"sessions.snapshots")
	addTool[listAgentsIn](b, "list_models",
		"List the Claude models available to spawn_agent (ids + display names).",
		"claude.listModels")
	addTool[listAgentsIn](b, "get_host_cwd",
		"Get the workspacer host process's current working directory — a sensible default base for new agents.",
		"app.getCwd")
	addTool[cwdIn](b, "list_resumable_sessions",
		"List prior Claude Code sessions for a directory that can be resumed (the resume picker), newest first.",
		"claude.sessionsForDir")

	// ── Spawn ──────────────────────────────────────────────────────────────
	b.group = "spawn"
	addSpawnTool(b, "spawn_agent",
		"Start a new coding-agent session in a directory (claude by default; codex/opencode/pi via provider) and return its sessionId. See help topic 'spawn' for labeling, nesting, and granting the new agent workspacer tools via toolScope.",
		"agents.spawn")
	addTool[createTerminalIn](b, "create_terminal",
		"Open a new shell terminal session. Returns the new sessionId; write to it with terminal_input.",
		"terminals.create")

	// ── Drive ──────────────────────────────────────────────────────────────
	b.group = "drive"
	addHubTool[sendMessageIn](b, "send_message",
		"Send a prompt/message to a running agent session.",
		"agents.sendMessage")
	addHubTool[approveIn](b, "approve",
		"Resolve a pending permission prompt for an agent: 'yes', 'no', or 'always'.",
		"claude.approve")
	addHubTool[answerIn](b, "answer",
		"Answer an agent's AskUserQuestion picker by option number, free text, or a list of answers.",
		"claude.answer")
	addHubTool[signalIn](b, "signal",
		"Send a POSIX signal to an agent session, e.g. SIGINT to interrupt or SIGTERM to stop it.",
		"claude.signal")
	addHubTool[gateIn](b, "set_approval_gate",
		"Turn an agent's approval gate on or off. When on, the agent pauses for permission before running tools (surfaced via list_agents / get_snapshot for you to approve).",
		"claude.gate")
	addTool[terminalInputIn](b, "terminal_input",
		"Type raw bytes into a session's terminal (PTY) — e.g. a command followed by a carriage return (\\r), or Ctrl-C (\\u0003).",
		"sessions.terminalInput")
	addTool[terminalResizeIn](b, "terminal_resize",
		"Resize a session's PTY grid (cols × rows). The PTY is shared, so this reflows the desktop pane too.",
		"sessions.terminalResize")

	// ── Filesystem (on the workspacer host) ────────────────────────────────
	b.group = "files"
	addTool[listDirIn](b, "list_dir",
		"List sub-directories of a host path (directories only, hidden skipped) — for choosing a working directory. Defaults to the user's home; returns { path, parent, home, dirs }.",
		"fs.listDir")
	addTool[listDirIn](b, "list_entries",
		"List files and directories at a host path (gitignore-aware), for an editor-style file tree.",
		"fs.listEntries")
	addTool[readFileIn](b, "read_file",
		"Read a UTF-8 text file on the workspacer host. Returns its contents.",
		"fs.read")
	addTool[writeFileIn](b, "write_file",
		"Write (create or overwrite) a UTF-8 text file on the workspacer host.",
		"fs.write")
	addTool[searchProjectIn](b, "search_project",
		"ripgrep a project directory for a query, returning matches grouped by file. Use for code search across the host project.",
		"search.project")

	// ── Config ─────────────────────────────────────────────────────────────
	b.group = "config"
	addTool[listAgentsIn](b, "get_config",
		"Get the full workspacer config (theme, keybindings, pane and session settings).",
		"config.get")
	addTool[listAgentsIn](b, "get_config_path",
		"Get the path to the workspacer config file on the host.",
		"config.getPath")
	addTool[listAgentsIn](b, "reload_config",
		"Re-read the config file from disk and return it.",
		"config.reload")
	addObjectTool(b, "save_config",
		"Persist a partial config patch (deep-merged into the current config). Pass only the keys to change, e.g. {\"ui\":{\"guiFontScale\":1.3}}.",
		"config.save")

	// ── Claude profiles ────────────────────────────────────────────────────
	b.group = "profiles"
	addTool[listAgentsIn](b, "list_profiles",
		"List configured Claude profiles (named CLAUDE_CONFIG_DIR + extra-args presets used when spawning agents).",
		"claude.profiles.list")
	addTool[addProfileIn](b, "add_profile",
		"Add a Claude profile. name is required; configDir and extraArgs optional.",
		"claude.profiles.add")
	addObjectTool(b, "update_profile",
		"Update a Claude profile. Pass { id, updates: { name?, configDir?, extraArgs? } }.",
		"claude.profiles.update")
	addTool[idIn](b, "remove_profile",
		"Remove a Claude profile by id.",
		"claude.profiles.remove")

	// ── Saved sessions (workspace arrangements) ────────────────────────────
	b.group = "sessions"
	addTool[listAgentsIn](b, "list_saved_sessions",
		"List saved workspace sessions (the session picker — saved pane/agent arrangements).",
		"sessions.list")
	addTool[filenameIn](b, "load_saved_session",
		"Load one saved workspace session by filename.",
		"sessions.load")
	addObjectTool(b, "save_saved_session",
		"Save the current workspace arrangement. Pass the session blob ({ name, tabs|agents, ... }).",
		"sessions.save")
	addTool[filenameIn](b, "delete_saved_session",
		"Delete a saved workspace session by filename.",
		"sessions.delete")

	// ── Layout templates ───────────────────────────────────────────────────
	b.group = "layouts"
	addTool[listAgentsIn](b, "list_layouts",
		"List saved layout templates (pane geometry presets).",
		"layouts.list")
	addObjectTool(b, "save_layout",
		"Save a layout template. Pass the layout blob ({ id?, name, ... }).",
		"layouts.save")
	addTool[idIn](b, "delete_layout",
		"Delete a layout template by id.",
		"layouts.delete")

	// ── Library (reusable prompts, skills, agents) ─────────────────────────
	b.group = "library"
	addTool[cwdIn](b, "list_library",
		"List reusable library items (prompts, skills, agents) — global plus, if cwd is given, that project's items.",
		"library.list")
	addObjectTool(b, "save_library",
		"Save a library item. Pass the item blob (scope, kind, id, name, body, …).",
		"library.save")
	addTool[libraryRemoveIn](b, "remove_library",
		"Remove a library item. Pass { scope: 'global'|'project'|'claude', id, cwd?, kind?: 'prompt'|'skill'|'agent' }.",
		"library.remove")

	// ── Analytics ──────────────────────────────────────────────────────────
	b.group = "analytics"
	addTool[listAgentsIn](b, "analytics_summary",
		"Get aggregate usage analytics across sessions (totals for tokens, cost, durations).",
		"analytics.summary")
	addTool[recentIn](b, "analytics_recent",
		"Get the most recent finished sessions with their per-session usage. Pass limit to cap the count.",
		"analytics.recent")

	// ── Notify ─────────────────────────────────────────────────────────────
	b.group = "notify"
	addTool[notifyIn](b, "notify",
		"Show a desktop notification on the workspacer machine.",
		"notifications.post")

	// ── Jobs (operator only — jobs.* matches no scoped tier's allowlist) ───
	//
	// The write here is propose_job, NOT jobs.upsert, and that asymmetry is the
	// whole safety story. An operator token is `*`, so the bus already treats
	// it as trusted and would answer jobs.upsert; what withholds it is that no
	// tool exists for it. jobs.propose lands the row disabled and stamped, and
	// the hub refuses to run a stamped row — so the worst a talked-into agent
	// achieves is an entry in a review list, not argv that fires every night
	// forever. Give an agent a tool for jobs.upsert and that guarantee is gone.
	b.group = "jobs"
	addTool[listAgentsIn](b, "list_jobs",
		"List the hub's scheduled jobs with their trigger, action, next run and last result. Rows with a proposedBy field are proposals awaiting the user's approval and never run.",
		"jobs.list")
	addTool[idIn](b, "job_history",
		"Get recent run records for one job id (status ok/error/skipped, plus an output or error tail).",
		"jobs.history")
	addObjectTool(b, "propose_job",
		"Propose a scheduled job for the user to approve — pass a job spec ({name, trigger, action}; see help topic \"jobs\"). It is saved DISABLED and cannot run until the user approves it in Settings → Jobs, so say so rather than implying it is scheduled.",
		"jobs.propose")
	addTool[idIn](b, "run_job",
		"Run an existing approved job now by id. Refused for a proposal the user has not approved yet.",
		"jobs.run")
	addTool[idIn](b, "remove_job",
		"Delete a job (or withdraw a proposal) by id, along with its run history.",
		"jobs.remove")

	// ── UI navigation (event-backed, explicit triage+ gate; see ui.go) ─────
	b.group = "ui"
	addUiTools(b)

	// ── Plugin-contributed tools (per-token grants; see plugins.go) ────────
	b.group = "plugins"
	for _, p := range plugins {
		for _, t := range p.Tools {
			addPluginTool(b, p.PluginID, t)
		}
	}

	addHelpTool(b)

	return s
}

// forward sends params to a hub capability and renders the JSON result as an MCP
// tool result. Shared by the typed and freeform tool registrars.
func forward(ctx context.Context, c *busclient.Client, method string, params any) (*mcp.CallToolResult, any, error) {
	res, err := c.Call(ctx, method, params)
	if err != nil {
		return &mcp.CallToolResult{
			IsError: true,
			Content: []mcp.Content{&mcp.TextContent{Text: err.Error()}},
		}, nil, nil
	}
	text := string(res)
	if text == "" || text == "null" {
		text = "ok"
	}
	return &mcp.CallToolResult{
		Content: []mcp.Content{&mcp.TextContent{Text: text}},
	}, nil, nil
}

// addTool registers one MCP tool that forwards its typed input to a hub
// capability and returns the capability's JSON result as text. In is the tool's
// input shape (which becomes its input schema); the output is passed through
// untyped, so no output schema is advertised. The tool registers only when the
// build's tier may call its hub method — the same allowlist the hub bus
// enforces on a scoped connection — and is recorded in the registry the help
// tool renders from.
func addTool[In any](b *build, name, desc, method string) {
	if !b.allowed(method) {
		return
	}
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: method, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in In) (*mcp.CallToolResult, any, error) {
			return forward(ctx, b.c, method, in)
		})
}

// addSpawnTool is addHubTool specialized to spawn_agent, because spawn carries
// the one input the facade must judge per SESSION rather than per tier: a
// `profile` dispatch request (profileId). The tier says whether you may spawn
// at all; the token record's profilesAllowed grant says which Claude accounts
// you may spawn UNDER — and the check has to live here, where the per-request
// record was resolved, because the hub only ever sees the facade's own trusted
// bus credential. A granted id is forwarded as-is and the hub (which trusts
// this facade's host-token connection) stamps `profileGranted` for the
// provider; an ungranted id is refused out loud, never silently degraded —
// dispatching "to the account with headroom" and landing on the default
// account is a capacity bug wearing a success result.
func addSpawnTool(b *build, name, desc, method string) {
	if !b.allowed(method) {
		return
	}
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: method, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in spawnAgentIn) (*mcp.CallToolResult, any, error) {
			if in.ProfileID != "" && !slices.Contains(b.profiles, in.ProfileID) {
				return &mcp.CallToolResult{
					IsError: true,
					Content: []mcp.Content{&mcp.TextContent{Text: fmt.Sprintf(
						"profile %q is not granted to this session token (profilesAllowed: %v). Omit profileId to spawn under the default account, or ask the workspacer user to bless this session with that profile.",
						in.ProfileID, b.profiles)}},
				}, nil, nil
			}
			// Full-access grant, enforced HERE for the SAME structural reason as
			// the profile check above: the hub stamps `yoloGranted` for the
			// facade's single trusted host-token connection no matter which
			// session is multiplexed over it, so a per-SESSION grant can only be
			// judged where the session's own record (b.yolo) was resolved. Unlike
			// the profile path this DEGRADES silently rather than refusing — it
			// mirrors the established "remote spawns never auto-bypass approvals"
			// clamp (the brain's spawn handler, hubCapabilities.ts), so an
			// ungranted worker starts with approvals on instead of failing. When
			// granted, skipPermissions rides through untouched → the hub stamps
			// yoloGranted → the provider honors it. (spawnAgentIn's only bypass
			// surface is SkipPermissions; there is no permissionMode field to
			// scrub.)
			if !b.yolo {
				in.SkipPermissions = false
			}
			m := method
			if peer := in.takeHub(); peer != "" {
				m = "hub:" + peer + "/" + method
			}
			return forward(ctx, b.c, m, in)
		})
}

// addObjectTool registers a tool whose entire arguments object is forwarded
// verbatim as the capability's params. For capabilities that take a free-form
// object (a config patch, a saved-session blob) too nested to model as a typed
// struct — the schema is an open object so the model can pass any shape.
func addObjectTool(b *build, name, desc, method string) {
	if !b.allowed(method) {
		return
	}
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: method, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in map[string]any) (*mcp.CallToolResult, any, error) {
			return forward(ctx, b.c, method, in)
		})
}

// Tool input shapes. Field json tags must match each hub capability's expected
// params; jsonschema tags become the per-field descriptions the model sees.

// listAgentsIn is the empty input shared by every no-argument tool.
type listAgentsIn struct{}

type transcriptIn struct {
	hubArg
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
}

type sessionIn struct {
	hubArg
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
}

type conversationIn struct {
	hubArg
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	SinceSeq  *int   `json:"sinceSeq,omitempty" jsonschema:"return only items after this sequence number; omit for the full history"`
}

type cwdIn struct {
	Cwd string `json:"cwd,omitempty" jsonschema:"a project/working directory on the host"`
}

type idIn struct {
	ID string `json:"id" jsonschema:"the target id"`
}

type filenameIn struct {
	Filename string `json:"filename" jsonschema:"the saved session's filename"`
}

type gateIn struct {
	hubArg
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	On        bool   `json:"on" jsonschema:"true to require approval before tools run, false to let the agent run freely"`
}

type terminalResizeIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Cols      int    `json:"cols" jsonschema:"terminal width in columns"`
	Rows      int    `json:"rows" jsonschema:"terminal height in rows"`
}

type listDirIn struct {
	Path string `json:"path,omitempty" jsonschema:"the host directory to list (defaults to the user's home)"`
}

type readFileIn struct {
	Path string `json:"path" jsonschema:"absolute path of the file to read on the host"`
}

type writeFileIn struct {
	Path     string `json:"path" jsonschema:"absolute path of the file to write on the host"`
	Contents string `json:"contents" jsonschema:"the new file contents"`
}

type searchProjectIn struct {
	Query         string `json:"query" jsonschema:"the search query"`
	Cwd           string `json:"cwd" jsonschema:"the project directory to search"`
	CaseSensitive bool   `json:"caseSensitive,omitempty" jsonschema:"match case (default false)"`
	WholeWord     bool   `json:"wholeWord,omitempty" jsonschema:"match whole words only (default false)"`
	Regex         bool   `json:"regex,omitempty" jsonschema:"treat the query as a regular expression (default false)"`
}

type addProfileIn struct {
	Name      string   `json:"name" jsonschema:"display name for the profile"`
	ConfigDir string   `json:"configDir,omitempty" jsonschema:"CLAUDE_CONFIG_DIR for this profile (optional)"`
	ExtraArgs []string `json:"extraArgs,omitempty" jsonschema:"extra CLI args passed to claude for this profile (optional)"`
}

type libraryRemoveIn struct {
	Scope string `json:"scope" jsonschema:"one of: global, project, claude"`
	ID    string `json:"id" jsonschema:"the library item id"`
	Cwd   string `json:"cwd,omitempty" jsonschema:"project directory (required for project/claude scope)"`
	Kind  string `json:"kind,omitempty" jsonschema:"one of: prompt, skill, agent"`
}

type recentIn struct {
	Limit int `json:"limit,omitempty" jsonschema:"max number of recent sessions to return"`
}

type spawnAgentIn struct {
	// Hub gets its own field (not the hubArg embed) for its distinct
	// description: a remote spawn's meaning differs from "this session lives
	// there", and the peer's clamp is worth stating where the model reads it.
	Hub             string   `json:"hub,omitempty" jsonschema:"the peer hub to spawn on (a hub name from list_agents rows); omit for this machine. The peer clamps remote spawns itself — permission bypass (skipPermissions) is refused on a peer spawn unless the peer's own hub trusts the federation link with the full-access grant"`
	Provider        string   `json:"provider,omitempty" jsonschema:"coding-agent backend to run: claude (default), codex, opencode, or pi"`
	Transport       string   `json:"transport,omitempty" jsonschema:"claude/codex only: 'stream' runs headless (GUI-only); omit for the default"`
	Cwd             string   `json:"cwd,omitempty" jsonschema:"working directory for the new agent (defaults to the user's home)"`
	Model           string   `json:"model,omitempty" jsonschema:"model id to use, e.g. claude-opus-4-8 (optional; provider-specific)"`
	Effort          string   `json:"effort,omitempty" jsonschema:"reasoning-effort level: low, medium, high, xhigh, or max (claude/codex)"`
	ProfileID       string   `json:"profileId,omitempty" jsonschema:"workspacer Claude profile id to dispatch under (optional; refused unless your session token's profilesAllowed grant lists this exact id — see list_profiles for ids)"`
	SkipPermissions bool     `json:"skipPermissions,omitempty" jsonschema:"start the agent with --dangerously-skip-permissions (honored only when your session's token carries the full-access grant — the hub verifies and stamps it; ungranted requests spawn with approvals on, and remote/federated peer spawns are re-judged by the peer's own hub)"`
	Label           string   `json:"label,omitempty" jsonschema:"a short human label for the new agent, shown as its name in the UI"`
	ParentSessionId string   `json:"parentSessionId,omitempty" jsonschema:"the spawning agent's own session id; set this so the new agent appears nested under you in the UI"`
	MCPFacade       bool     `json:"mcpFacade,omitempty" jsonschema:"legacy: give the new agent the FULL workspacer tool set (operator tier); prefer toolScope"`
	ToolScope       string   `json:"toolScope,omitempty" jsonschema:"give the new agent the workspacer tools at a tier: view (observe-only — right for summarizer workers), triage (view + approve/reply/interrupt), or operator (everything)"`
	PluginTools     []string `json:"pluginTools,omitempty" jsonschema:"plugin ids whose contributed tools the new agent may use (requires toolScope); omit for none"`
}

// takeHub implements hubRouted for spawn_agent's own hub field.
func (in *spawnAgentIn) takeHub() string {
	peer := in.Hub
	in.Hub = ""
	return peer
}

type createTerminalIn struct {
	Shell string `json:"shell,omitempty" jsonschema:"shell to run (defaults to the platform default shell)"`
	Cwd   string `json:"cwd,omitempty" jsonschema:"working directory (defaults to the user's home)"`
	Cols  int    `json:"cols,omitempty" jsonschema:"initial terminal width in columns"`
	Rows  int    `json:"rows,omitempty" jsonschema:"initial terminal height in rows"`
}

type sendMessageIn struct {
	hubArg
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Text      string `json:"text" jsonschema:"the prompt/message to send to the agent"`
}

type approveIn struct {
	hubArg
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Decision  string `json:"decision" jsonschema:"one of: yes, no, always"`
	Reason    string `json:"reason,omitempty" jsonschema:"optional reason to record with the decision"`
}

type answerIn struct {
	hubArg
	SessionID string   `json:"sessionId" jsonschema:"the target session id"`
	Option    *int     `json:"option,omitempty" jsonschema:"the numeric option to pick"`
	Text      *string  `json:"text,omitempty" jsonschema:"a free-text answer"`
	Answers   []string `json:"answers,omitempty" jsonschema:"a list of answers for a multi-part question"`
}

type signalIn struct {
	hubArg
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Signal    string `json:"signal" jsonschema:"signal name, e.g. SIGINT or SIGTERM"`
}

type terminalInputIn struct {
	SessionID string `json:"sessionId" jsonschema:"the target session id"`
	Data      string `json:"data" jsonschema:"raw bytes to write to the PTY"`
}

type notifyIn struct {
	Title string `json:"title,omitempty" jsonschema:"notification title"`
	Body  string `json:"body,omitempty" jsonschema:"notification body"`
}
