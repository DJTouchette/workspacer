package main

// The capability handlers — the headless "brain". Each maps a hub capability
// (the same method names the Electron app registers in hubCapabilities.ts, so
// callers like the MCP facade and the web client see an identical surface) onto
// claudemon HTTP calls plus profile/argv logic. Running this daemon makes these
// capabilities available on the bus WITHOUT the desktop app — which is what lets
// a TUI or any client mirror the app instead of re-implementing it.

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
)

// registry holds the dependencies the handlers close over and dispatches calls
// by method name.
type registry struct {
	cm    *claudemonClient
	cfg   *configService
	store *sessionStore // live session store (full scope only); nil → proxy claudemon
	meta  *metaStore    // spawn metadata (label/parent/supervisor) for enrichment
	term  *terminalHub  // PTY-over-bus forwarders (full scope only)
	vis   *visibility   // shared desktop fleet-visibility rule; nil → show everything
}

// visibleSnapshots is the live store filtered by the shared desktop visibility
// rule (visibility.go) — the one list agents.list and sessions.snapshots serve
// so every bus client sees the desktop sidebar's fleet.
func (r *registry) visibleSnapshots(ctx context.Context) []json.RawMessage {
	snaps := r.store.all()
	if r.vis == nil {
		return snaps
	}
	return r.vis.filter(ctx, snaps)
}

func newRegistry(cm *claudemonClient) *registry {
	return &registry{cm: cm, cfg: newConfigService()}
}

// methods is the set of capabilities this provider registers on the bus. Names
// match the app's hubCapabilities.ts so callers see one identical surface.
func (r *registry) methods() []string {
	return []string{
		// agents + sessions (claudemon-backed)
		"agents.list",
		"agents.spawn",
		"agents.sendMessage",
		"terminals.create",
		"claude.approve",
		"claude.answer",
		"claude.signal",
		"claude.gate",
		"sessions.transcript",
		"sessions.conversation",
		"sessions.snapshots",
		"sessions.snapshot",
		"sessions.terminalInput",
		"sessions.terminalResize",
		"sessions.attachTerminal",
		"sessions.terminalKeepalive",
		"sessions.detachTerminal",
		// catalogs + config (file-backed)
		"claude.profiles.list",
		"claude.profiles.add",
		"claude.profiles.update",
		"claude.profiles.remove",
		"claude.listModels",
		// provider discovery: model catalog (relayed to claudemon) + PATH detection.
		"providers.listModels",
		"providers.checkAll",
		"config.get",
		"config.reload",
		"config.getPath",
		"config.save",
		"layouts.list",
		"layouts.save",
		"layouts.delete",
		"sessions.list",
		"sessions.load",
		"sessions.save",
		"sessions.delete",
		"library.list",
		"library.save",
		"library.remove",
		"claude.sessionsForDir",
		// host
		"app.getCwd",
		"app.supervisorHome",
		"fs.listDir",
		"fs.listEntries",
		"fs.read",
		"fs.write",
		"search.project",
		"notifications.post",
		// analytics: the desktop owns the real data (a local SQLite session-history
		// store fed by the app's hook accounting). Headless, there's no such store,
		// so the brain registers explicit empty-result stubs — a web client asking
		// for analytics degrades to an empty dashboard instead of "no provider".
		"analytics.summary",
		"analytics.recent",
	}
}

// catalogMethods is the file-backed "source of truth" subset: config, profiles,
// library, layouts, saved sessions, models, session discovery, and host file
// reads. These are the capabilities the brain owns when it runs *alongside* the
// desktop app (which keeps the live/enriched agent + streaming ones). Running
// with --scope catalog registers only these, so there's exactly one provider per
// method on the bus (the router is single-owner). The handler dispatch still
// serves every method — scope only controls what's registered.
func (r *registry) catalogMethods() []string {
	return []string{
		"config.get", "config.reload", "config.getPath", "config.save",
		"claude.listModels",
		"claude.profiles.list", "claude.profiles.add", "claude.profiles.update", "claude.profiles.remove",
		"library.list", "library.save", "library.remove",
		"layouts.list", "layouts.save", "layouts.delete",
		"sessions.list", "sessions.load", "sessions.save", "sessions.delete",
		"claude.sessionsForDir",
		"fs.listDir", "fs.read", "fs.write", "fs.listEntries",
	}
}

// methodsForScope selects the registration set. "catalog" → the file-backed
// subset (run alongside the app); anything else → the full surface (headless).
func (r *registry) methodsForScope(scope string) []string {
	if scope == "catalog" {
		return r.catalogMethods()
	}
	return r.methods()
}

// handle dispatches one capability call.
func (r *registry) handle(ctx context.Context, method string, params json.RawMessage) (json.RawMessage, error) {
	switch method {
	case "agents.list":
		if r.store != nil {
			return jsonResult(r.visibleSnapshots(ctx))
		}
		return r.cm.listSessions(ctx)
	case "agents.spawn":
		return r.spawn(ctx, params)
	case "agents.sendMessage":
		return r.sendMessage(ctx, params)
	case "terminals.create":
		return r.terminalsCreate(ctx, params)
	case "claude.approve":
		return r.approve(ctx, params)
	case "claude.answer":
		return r.answer(ctx, params)
	case "claude.signal":
		return r.signal(ctx, params)
	case "claude.gate":
		return r.gate(ctx, params)
	case "sessions.transcript":
		return r.transcript(ctx, params)
	case "sessions.conversation":
		return r.conversation(ctx, params)
	case "sessions.snapshots":
		if r.store != nil {
			return jsonResult(r.visibleSnapshots(ctx))
		}
		return r.cm.listSessions(ctx)
	case "sessions.snapshot":
		return r.snapshot(ctx, params)
	case "sessions.terminalInput":
		return r.terminalInput(ctx, params)
	case "sessions.terminalResize":
		return r.terminalResize(ctx, params)
	case "sessions.attachTerminal":
		return r.attachTerminal(params)
	case "sessions.terminalKeepalive":
		return r.terminalKeepalive(params)
	case "sessions.detachTerminal":
		return r.detachTerminal(params)
	case "claude.profiles.list":
		return jsonResult(loadProfiles())
	case "claude.profiles.add":
		return r.profilesAdd(params)
	case "claude.profiles.update":
		return r.profilesUpdate(params)
	case "claude.profiles.remove":
		return r.profilesRemove(params)
	case "claude.listModels":
		return jsonResult(r.listModels(ctx))
	case "providers.listModels":
		return r.providersListModels(ctx, params)
	case "providers.checkAll":
		return r.providersCheckAll()
	case "config.get":
		return jsonResult(r.cfg.get())
	case "config.reload":
		return jsonResult(r.cfg.reload())
	case "config.getPath":
		return jsonResult(r.cfg.path())
	case "config.save":
		var partial map[string]any
		if err := unmarshal(params, &partial); err != nil {
			return nil, err
		}
		return jsonResult(r.cfg.save(partial))
	case "layouts.list":
		return jsonResult(listLayouts())
	case "layouts.save":
		return r.layoutsSave(params)
	case "layouts.delete":
		return r.layoutsDelete(params)
	case "sessions.list":
		return jsonResult(listSavedSessions())
	case "sessions.load":
		return r.savedSessionLoad(params)
	case "sessions.save":
		return r.savedSessionSave(params)
	case "sessions.delete":
		return r.savedSessionDelete(params)
	case "library.list":
		var p struct {
			Cwd string `json:"cwd"`
		}
		if err := unmarshal(params, &p); err != nil {
			return nil, err
		}
		return jsonResult(listLibrary(p.Cwd))
	case "library.save":
		var in libraryInput
		if err := unmarshal(params, &in); err != nil {
			return nil, err
		}
		item, err := saveLibrary(in)
		if err != nil {
			return nil, err
		}
		return jsonResult(item)
	case "library.remove":
		var p struct {
			Scope string `json:"scope"`
			ID    string `json:"id"`
			Cwd   string `json:"cwd"`
			Kind  string `json:"kind"`
		}
		if err := unmarshal(params, &p); err != nil {
			return nil, err
		}
		if p.Scope == "" || p.ID == "" {
			return nil, fmt.Errorf("library.remove requires { scope, id }")
		}
		removeLibrary(p.Scope, p.ID, p.Cwd, p.Kind)
		return okResult()
	case "claude.sessionsForDir":
		var p struct {
			Cwd string `json:"cwd"`
		}
		if err := unmarshal(params, &p); err != nil {
			return nil, err
		}
		if p.Cwd == "" {
			return nil, fmt.Errorf("claude.sessionsForDir requires { cwd }")
		}
		return jsonResult(listClaudeSessionsForDir(p.Cwd))
	case "app.getCwd":
		return r.getCwd()
	case "app.supervisorHome":
		return jsonResult(supervisorHome())
	case "fs.listDir":
		return r.fsListDir(params)
	case "fs.listEntries":
		return r.fsListEntries(params)
	case "fs.read":
		return r.fsRead(params)
	case "fs.write":
		return r.fsWrite(params)
	case "search.project":
		return r.searchProject(ctx, params)
	case "notifications.post":
		return r.notify(params)
	case "analytics.summary":
		return analyticsSummaryStub()
	case "analytics.recent":
		return analyticsRecentStub()
	default:
		return nil, fmt.Errorf("unknown method %q", method)
	}
}

// analyticsSummaryStub is the headless stand-in for the desktop's
// analytics.summary. The real breakdown comes from the app's local SQLite
// session-history store, which the brain has no access to — so it returns a
// well-formed but empty AnalyticsSummary (matching the shape the renderer's
// AnalyticsPane consumes) plus an "unavailable":"headless" marker. A web client
// then shows an empty dashboard and, if it cares, can note analytics needs the
// desktop, rather than erroring on a missing provider.
func analyticsSummaryStub() (json.RawMessage, error) {
	return jsonResult(map[string]any{
		"totals": map[string]any{
			"sessions": 0, "costUSD": 0, "inputTokens": 0, "outputTokens": 0,
			"toolCalls": 0, "durationMs": 0, "workflowRuns": 0,
		},
		"byDay":       []any{},
		"byProject":   []any{},
		"byModel":     []any{},
		"byProvider":  []any{},
		"unavailable": "headless",
	})
}

// analyticsRecentStub is the headless stand-in for analytics.recent: an empty
// session list (the renderer expects an array and renders nothing for []).
func analyticsRecentStub() (json.RawMessage, error) {
	return jsonResult([]any{})
}

// ── param shapes (match the MCP facade / app capability inputs) ─────────────

type spawnParams struct {
	// Provider backend: claude (default) | codex | opencode | pi. Non-claude
	// providers — and claude on the 'stream' transport — go through claudemon's
	// /sessions/spawn-managed; PTY claude keeps the classic argv spawn.
	Provider string `json:"provider"`
	// Claude: 'pty' | 'stream' (omitted = the config's claude.transport, then
	// pty). Codex: 'stream' spawns headless (GUI-only, daemon-owned thread).
	Transport string `json:"transport"`
	Cwd       string `json:"cwd"`
	Model     string `json:"model"`
	// Reasoning-effort level (codex `model_reasoning_effort`); others ignore it.
	Effort    string `json:"effort"`
	ProfileID string `json:"profileId"`
	// Claude permission mode (default/acceptEdits/plan/…). Bypass modes are
	// clamped off for bus callers — see the security rule in spawn().
	PermissionMode  string `json:"permissionMode"`
	SkipPermissions bool   `json:"skipPermissions"`
	ResumeSessionID string `json:"resumeSessionId"`
	Cols            int    `json:"cols"`
	Rows            int    `json:"rows"`
	Label           string `json:"label"`
	ParentSessionID string `json:"parentSessionId"`
	Supervisor      bool   `json:"supervisor"`
}

type sessionParam struct {
	SessionID string `json:"sessionId"`
}

func (r *registry) spawn(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p spawnParams
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}

	// SECURITY (mirrors hubCapabilities.ts agents.spawn): this capability is the
	// REMOTE/web/MCP spawn path. Driving an agent is already code execution on
	// the host, but we refuse to let a bus caller silently auto-bypass every
	// approval (`--dangerously-skip-permissions` / bypass-sandbox). Approvals
	// still surface and can be answered remotely; a YOLO agent must be started
	// locally. So skipPermissions is forced off, and a bypass permissionMode is
	// dropped (other modes pass through).
	if p.SkipPermissions || p.PermissionMode == "bypassPermissions" || p.PermissionMode == "yolo" {
		log.Printf("brain: agents.spawn: ignoring permission bypass from a bus client — remote spawns never auto-bypass approvals.")
	}
	p.SkipPermissions = false
	if p.PermissionMode == "bypassPermissions" || p.PermissionMode == "yolo" {
		p.PermissionMode = ""
	}

	cwd := normalizeCwd(p.Cwd)
	if cwd == "" {
		home, _ := os.UserHomeDir()
		cwd = home
	}

	// Managed (Tier-2) backend — Codex / OpenCode / Pi run through claudemon's
	// adapter, not a Claude PTY. Same dispatch split as the desktop's
	// agents.spawn so this path can't silently fall back to spawning Claude.
	provider := p.Provider
	if provider == "" {
		provider = "claude"
	}
	if provider != "claude" {
		return r.spawnManagedSession(ctx, provider, cwd, p)
	}

	// Claude on the 'stream' transport is managed too (claudemon's headless
	// stream-json adapter, no PTY). Mirror the desktop's default resolution: an
	// explicit transport wins, else the config's claude.transport, else pty.
	transport := p.Transport
	if transport == "" {
		transport = r.claudeTransportDefault()
	}
	if transport == "stream" {
		return r.spawnManagedSession(ctx, "claude", cwd, p)
	}

	prof := getProfile(p.ProfileID)

	// Resume reopens an existing transcript; a fresh spawn pins a new id so our
	// id, claude's id, and the transcript filename all agree.
	resume := p.ResumeSessionID != ""
	sessionID := p.ResumeSessionID
	if !resume {
		var err error
		if sessionID, err = newSessionID(); err != nil {
			return nil, err
		}
	}

	// Record spawn metadata before the session registers, so the live store's
	// enricher picks up the name/parent the moment claudemon reports SessionStart.
	if r.meta != nil && (p.Label != "" || p.ParentSessionID != "" || p.Supervisor) {
		r.meta.set(sessionID, spawnMeta{Label: p.Label, ParentSessionID: p.ParentSessionID, IsSupervisor: p.Supervisor})
	}

	cols, rows := p.Cols, p.Rows
	if cols == 0 {
		cols = 120
	}
	if rows == 0 {
		rows = 32
	}

	id, err := r.cm.spawn(ctx, spawnReq{
		Argv:      buildArgv(prof, p.Model, p.Effort, p.SkipPermissions, p.PermissionMode, sessionID, resume),
		Cwd:       cwd,
		Cols:      cols,
		Rows:      rows,
		Env:       buildEnv(prof),
		SessionID: sessionID,
	})
	if err != nil {
		return nil, err
	}
	return jsonResult(map[string]string{"sessionId": id})
}

// spawnManagedSession launches an adapter-driven session via claudemon's
// POST /sessions/spawn-managed — Codex/OpenCode/Pi, plus Claude on the headless
// stream-json transport. Mirrors the desktop's spawnManagedAgent
// (managedSpawn.ts): a resume rides the prior id (managed ids are not
// re-pinnable), a fresh spawn pins a new one; codex's 'stream' transport is
// forwarded on the wire; claude-stream carries permission_mode + resume + the
// profile's env/extra argv and — deliberately — no wire `transport` key
// (spawn-managed claude IS the stream adapter). The caller has already clamped
// off every bypass, so `yolo` is always false on this path.
func (r *registry) spawnManagedSession(ctx context.Context, provider, cwd string, p spawnParams) (json.RawMessage, error) {
	isClaudeStream := provider == "claude"
	isCodexStream := provider == "codex" && p.Transport == "stream"

	sessionID := p.ResumeSessionID
	if sessionID == "" {
		var err error
		if sessionID, err = newSessionID(); err != nil {
			return nil, err
		}
	}
	if r.meta != nil && (p.Label != "" || p.ParentSessionID != "" || p.Supervisor) {
		r.meta.set(sessionID, spawnMeta{Label: p.Label, ParentSessionID: p.ParentSessionID, IsSupervisor: p.Supervisor})
	}

	req := spawnManagedReq{
		Provider:  provider,
		Cwd:       cwd,
		Model:     p.Model,
		Effort:    p.Effort,
		Bin:       r.resolveSpawnBin(provider),
		SessionID: sessionID,
	}
	if isCodexStream {
		// Codex mirrors Claude's stream transport: 'stream' spawns headless
		// (GUI-only, daemon-owned thread). Must ride on the wire or a remote
		// headless spawn silently downgrades to the hybrid PTY session.
		req.Transport = "stream"
	}
	// Resume: codex rejoins the prior life's app-server thread; claude-stream
	// passes `--resume`. opencode/pi carry no resume on the wire — matching the
	// desktop dispatch.
	if (provider == "codex" || isClaudeStream) && p.ResumeSessionID != "" {
		req.Resume = p.ResumeSessionID
	}
	if isClaudeStream {
		// Claude keeps its full permission-mode vocabulary; an absent mode
		// resolves to 'default', same as the desktop (bypass never survives the
		// clamp above). Profile parity with the PTY path: CLAUDE_CONFIG_DIR +
		// extra argv ride the payload's claude-only env/extra_args fields.
		mode := p.PermissionMode
		if mode == "" {
			mode = "default"
		}
		req.PermissionMode = mode
		if prof := getProfile(p.ProfileID); prof != nil {
			if env := buildEnv(prof); len(env) > 0 {
				req.Env = env
			}
			if len(prof.ExtraArgs) > 0 {
				req.ExtraArgs = prof.ExtraArgs
			}
		}
	}
	id, err := r.cm.spawnManaged(ctx, req)
	if err != nil {
		return nil, err
	}
	return jsonResult(map[string]string{"sessionId": id})
}

// claudeTransportDefault reads config.claude.transport — the same default the
// desktop applies when a spawn names no transport (hubCapabilities.ts:
// reqTransport ?? config.claude.transport ?? 'pty').
func (r *registry) claudeTransportDefault() string {
	claude, _ := r.cfg.get()["claude"].(map[string]any)
	if t := str(claude["transport"]); t != "" {
		return t
	}
	return "pty"
}

func (r *registry) sendMessage(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Text      string `json:"text"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" || p.Text == "" {
		return nil, fmt.Errorf("agents.sendMessage requires { sessionId, text }")
	}
	// claudemon's /message settles + verifies delivery itself and QUEUES the
	// text when the session is mid-turn or holding a dialog — a 409 now only
	// means the session has ended. The old "type into the PTY" fallback fired
	// exactly then, silently dropping the text into a dead terminal (the
	// classic stuck mobile send) — surface the failure instead.
	ok, err := r.cm.submitMessage(ctx, p.SessionID, p.Text)
	if err != nil {
		return nil, err
	}
	if !ok {
		return nil, fmt.Errorf("session has ended and cannot accept messages")
	}
	return okResult()
}

func (r *registry) approve(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Decision  string `json:"decision"`
		Reason    string `json:"reason"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" || p.Decision == "" {
		return nil, fmt.Errorf("claude.approve requires { sessionId, decision: 'yes'|'no'|'always' }")
	}
	if err := r.cm.approve(ctx, p.SessionID, p.Decision, p.Reason); err != nil {
		return nil, err
	}
	return okResult()
}

// answer drives an AskUserQuestion picker by typing into the PTY (the option
// number, free text, or each answer of a multi-part question, followed by
// Enter) rather than the mode-gated /answer endpoint, which requires
// mode=Question and races with concurrent hook events. This mirrors the desktop
// ClaudePane handleAnswer exactly, so it lands whether the picker arrived via
// PreToolUse or mid-stream.
func (r *registry) answer(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string   `json:"sessionId"`
		Option    *int     `json:"option"`
		Text      *string  `json:"text"`
		Answers   []string `json:"answers"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("claude.answer requires { sessionId }")
	}
	switch {
	case p.Option != nil:
		if err := r.cm.input(ctx, p.SessionID, fmt.Sprintf("%d\r", *p.Option)); err != nil {
			return nil, err
		}
	case p.Text != nil:
		if err := r.cm.input(ctx, p.SessionID, *p.Text+"\r"); err != nil {
			return nil, err
		}
	case p.Answers != nil:
		for _, a := range p.Answers {
			if err := r.cm.input(ctx, p.SessionID, a+"\r"); err != nil {
				return nil, err
			}
		}
	default:
		return nil, fmt.Errorf("claude.answer requires one of { option, text, answers }")
	}
	return okResult()
}

func (r *registry) signal(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Signal    string `json:"signal"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" || p.Signal == "" {
		return nil, fmt.Errorf("claude.signal requires { sessionId, signal }")
	}
	if err := r.cm.signal(ctx, p.SessionID, p.Signal); err != nil {
		return nil, err
	}
	return okResult()
}

func (r *registry) transcript(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		// Optional: resolve historical sessions (unknown to claudemon) from
		// the on-disk JSONL for this working directory.
		Cwd string `json:"cwd"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("sessions.transcript requires { sessionId }")
	}
	return r.cm.transcript(ctx, p.SessionID, p.Cwd)
}

func (r *registry) conversation(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		SinceSeq  *int   `json:"sinceSeq"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("sessions.conversation requires { sessionId }")
	}
	return r.cm.conversation(ctx, p.SessionID, p.SinceSeq)
}

func (r *registry) gate(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		On        bool   `json:"on"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("claude.gate requires { sessionId, on }")
	}
	return r.cm.gate(ctx, p.SessionID, p.On)
}

func (r *registry) snapshot(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p sessionParam
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("sessions.snapshot requires { sessionId }")
	}
	if r.store != nil {
		if snap, ok := r.store.get(p.SessionID); ok {
			return snap, nil
		}
	}
	// Not in the store (e.g. catalog scope, or a stopped session fetched
	// explicitly): relay claudemon's row with the same desktop-shape overlay
	// the store applies, so the caller sees one consistent snapshot shape.
	snap, err := r.cm.getSession(ctx, p.SessionID)
	if err != nil {
		return nil, err
	}
	return compatSnapshot(snap), nil
}

// terminalsCreate opens a shell PTY in claudemon — the headless counterpart of
// the app's terminals.create. Defaults the shell to $SHELL (or /bin/sh) and the
// cwd to home, like the app's detectDefaultShell.
func (r *registry) terminalsCreate(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Shell string `json:"shell"`
		Cwd   string `json:"cwd"`
		Cols  int    `json:"cols"`
		Rows  int    `json:"rows"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	shell := p.Shell
	if shell == "" {
		if shell = os.Getenv("SHELL"); shell == "" {
			shell = "/bin/sh"
		}
	}
	cwd := normalizeCwd(p.Cwd)
	if cwd == "" {
		cwd, _ = os.UserHomeDir()
	}
	cols, rows := p.Cols, p.Rows
	if cols == 0 {
		cols = 120
	}
	if rows == 0 {
		rows = 32
	}
	// No session_id pinned: a shell has no claude transcript to align with.
	id, err := r.cm.spawn(ctx, spawnReq{Argv: []string{shell}, Cwd: cwd, Cols: cols, Rows: rows})
	if err != nil {
		return nil, err
	}
	return jsonResult(map[string]string{"sessionId": id})
}

func (r *registry) terminalInput(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Data      string `json:"data"`
		BytesB64  string `json:"bytesB64"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("sessions.terminalInput requires { sessionId, data|bytesB64 }")
	}
	// Raw keystrokes come as base64 bytes; plain text uses the text path.
	var err error
	if p.BytesB64 != "" {
		err = r.cm.inputBytes(ctx, p.SessionID, p.BytesB64)
	} else {
		err = r.cm.input(ctx, p.SessionID, p.Data)
	}
	if err != nil {
		return nil, err
	}
	return okResult()
}

func (r *registry) terminalResize(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Cols      int    `json:"cols"`
		Rows      int    `json:"rows"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" || p.Cols == 0 || p.Rows == 0 {
		return nil, fmt.Errorf("sessions.terminalResize requires { sessionId, cols, rows }")
	}
	if err := r.cm.resize(ctx, p.SessionID, p.Cols, p.Rows); err != nil {
		return nil, err
	}
	return okResult()
}

func (r *registry) profilesAdd(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Name       string   `json:"name"`
		ConfigDir  string   `json:"configDir"`
		ExtraArgs  []string `json:"extraArgs"`
		MCPItemIDs []string `json:"mcpItemIds"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	prof, err := addProfile(p.Name, p.ConfigDir, p.ExtraArgs, p.MCPItemIDs)
	if err != nil {
		return nil, err
	}
	return jsonResult(prof)
}

func (r *registry) profilesUpdate(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		ID      string        `json:"id"`
		Updates profileUpdate `json:"updates"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.ID == "" {
		return nil, fmt.Errorf("claude.profiles.update requires { id, updates }")
	}
	prof, err := updateProfile(p.ID, p.Updates)
	if err != nil {
		return nil, err
	}
	return jsonResult(prof)
}

func (r *registry) profilesRemove(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		ID string `json:"id"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.ID == "" {
		return nil, fmt.Errorf("claude.profiles.remove requires { id }")
	}
	if err := removeProfile(p.ID); err != nil {
		return nil, err
	}
	return okResult()
}

func (r *registry) layoutsSave(raw json.RawMessage) (json.RawMessage, error) {
	var input map[string]any
	if err := unmarshal(raw, &input); err != nil {
		return nil, err
	}
	if str(input["name"]) == "" && str(input["id"]) == "" {
		return nil, fmt.Errorf("layouts.save requires { name }")
	}
	layout, err := saveLayout(input)
	if err != nil {
		return nil, err
	}
	return jsonResult(layout)
}

func (r *registry) layoutsDelete(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		ID string `json:"id"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.ID == "" {
		return nil, fmt.Errorf("layouts.delete requires { id }")
	}
	removeLayout(p.ID)
	return okResult()
}

func (r *registry) savedSessionLoad(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Filename string `json:"filename"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.Filename == "" {
		return nil, fmt.Errorf("sessions.load requires { filename }")
	}
	s := loadSavedSession(p.Filename)
	if s == nil {
		return json.RawMessage("null"), nil // matches the app's null-on-missing
	}
	return jsonResult(s)
}

// savedSessionSave persists the session blob. It mirrors the app's two branches
// (agent-centric vs legacy tabs) and stamps the timestamp, but does not perform
// the desktop's terminal-cwd enrichment — that relies on the GUI's in-process
// pty→cwd map; a headless caller passes cwds it already knows.
func (r *registry) savedSessionSave(raw json.RawMessage) (json.RawMessage, error) {
	var p map[string]any
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	name := str(p["name"])
	data := map[string]any{"name": name, "timestamp": nowISO()}
	if agents, ok := p["agents"].([]any); ok {
		if p["activeAgentId"] != nil {
			data["activeAgentId"] = p["activeAgentId"]
		}
		data["agents"] = agents
	} else {
		if p["activeTabId"] != nil {
			data["activeTabId"] = p["activeTabId"]
		}
		tabs := p["tabs"]
		if tabs == nil {
			tabs = []any{}
		}
		data["tabs"] = tabs
	}
	filename, err := saveSavedSession(name, data)
	if err != nil {
		return nil, err
	}
	return jsonResult(filename)
}

func (r *registry) savedSessionDelete(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Filename string `json:"filename"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.Filename == "" {
		return nil, fmt.Errorf("sessions.delete requires { filename }")
	}
	deleteSavedSession(p.Filename)
	return okResult()
}

func (r *registry) attachTerminal(raw json.RawMessage) (json.RawMessage, error) {
	var p sessionParam
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("sessions.attachTerminal requires { sessionId }")
	}
	if r.term == nil {
		return nil, fmt.Errorf("PTY streaming unavailable (catalog scope)")
	}
	r.term.attach(p.SessionID)
	return okResult()
}

func (r *registry) terminalKeepalive(raw json.RawMessage) (json.RawMessage, error) {
	var p sessionParam
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("sessions.terminalKeepalive requires { sessionId }")
	}
	ok := r.term != nil && r.term.keepalive(p.SessionID)
	return jsonResult(map[string]bool{"ok": ok})
}

func (r *registry) detachTerminal(raw json.RawMessage) (json.RawMessage, error) {
	var p sessionParam
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("sessions.detachTerminal requires { sessionId }")
	}
	if r.term != nil {
		r.term.detach(p.SessionID)
	}
	return okResult()
}

func (r *registry) getCwd() (json.RawMessage, error) {
	cwd, err := os.Getwd()
	if err != nil {
		return nil, err
	}
	return jsonResult(cwd)
}

func (r *registry) fsListDir(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Path string `json:"path"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	res, err := listHostDir(p.Path)
	if err != nil {
		return nil, err
	}
	return jsonResult(res)
}

func (r *registry) fsRead(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Path string `json:"path"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.Path == "" {
		return nil, fmt.Errorf("fs.read requires a path")
	}
	res, err := readTextFile(p.Path)
	if err != nil {
		return nil, err
	}
	return jsonResult(res)
}

func (r *registry) fsListEntries(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Path string `json:"path"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.Path == "" {
		return nil, fmt.Errorf("fs.listEntries requires a path")
	}
	res, err := listEntries(p.Path)
	if err != nil {
		return nil, err
	}
	return jsonResult(res)
}

func (r *registry) searchProject(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var opts searchOpts
	if err := unmarshal(raw, &opts); err != nil {
		return nil, err
	}
	if opts.Query == "" || opts.Cwd == "" {
		return nil, fmt.Errorf("search.project requires { query, cwd }")
	}
	res, err := searchProject(ctx, opts)
	if err != nil {
		return nil, err
	}
	return jsonResult(res)
}

// notify is best-effort headless: there's no desktop to raise an OS
// notification, so we log it and ack. (A connected GUI still gets its own.)
func (r *registry) notify(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Title string `json:"title"`
		Body  string `json:"body"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	log.Printf("brain: notification: %s — %s", firstNonEmpty(p.Title, "workspacer"), p.Body)
	return okResult()
}

func (r *registry) fsWrite(raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Path     string `json:"path"`
		Contents string `json:"contents"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.Path == "" {
		return nil, fmt.Errorf("fs.write requires a path")
	}
	if err := writeHostFile(p.Path, p.Contents); err != nil {
		return nil, err
	}
	return okResult()
}

// ── helpers ─────────────────────────────────────────────────────────────────

// unmarshal decodes params, tolerating an empty/null body as an empty object so
// no-arg capabilities (and optional fields) don't error.
func unmarshal(raw json.RawMessage, out any) error {
	if len(raw) == 0 || string(raw) == "null" {
		return nil
	}
	if err := json.Unmarshal(raw, out); err != nil {
		return fmt.Errorf("invalid params: %w", err)
	}
	return nil
}

func jsonResult(v any) (json.RawMessage, error) {
	b, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	return json.RawMessage(b), nil
}

func okResult() (json.RawMessage, error) {
	return json.RawMessage(`{"ok":true}`), nil
}
