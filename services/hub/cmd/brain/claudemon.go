package main

// Thin HTTP client for the claudemon daemon's REST API (services/claudemon).
// claudemon owns the engine — PTYs and Claude sessions — and exposes the full
// surface over loopback HTTP. The brain turns high-level bus capabilities into
// these low-level calls, exactly as the Electron app's claudemonSessionClient
// and the TUI's claudemon.rs do. Endpoints mirror services/claudemon/src/daemon/api.rs.

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

type claudemonClient struct {
	base string
	http *http.Client
}

func newClaudemonClient(base string) *claudemonClient {
	return &claudemonClient{base: base, http: &http.Client{Timeout: 30 * time.Second}}
}

// getRaw fetches a path and returns the response body verbatim, so capabilities
// that just relay claudemon's JSON (list/transcript/conversation) stay faithful
// to what claudemon serves — no lossy re-shaping in the brain.
func (c *claudemonClient) getRaw(ctx context.Context, path string) (json.RawMessage, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.base+path, nil)
	if err != nil {
		return nil, err
	}
	resp, err := c.http.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode >= 400 {
		return nil, fmt.Errorf("claudemon GET %s: %s: %s", path, resp.Status, string(body))
	}
	return json.RawMessage(body), nil
}

// postJSON sends a JSON body and decodes the JSON response into out (out may be
// nil to discard it). A 4xx/5xx is surfaced as an error including the body.
func (c *claudemonClient) postJSON(ctx context.Context, path string, body any, out any) error {
	buf, err := json.Marshal(body)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.base+path, bytes.NewReader(buf))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if resp.StatusCode >= 400 {
		return fmt.Errorf("claudemon POST %s: %s: %s", path, resp.Status, string(raw))
	}
	if out != nil && len(raw) > 0 {
		return json.Unmarshal(raw, out)
	}
	return nil
}

func (c *claudemonClient) listSessions(ctx context.Context) (json.RawMessage, error) {
	return c.getRaw(ctx, "/sessions")
}

// transcript fetches a session's parsed transcript. A non-empty cwd lets
// claudemon resolve sessions it isn't tracking from the on-disk JSONL under
// ~/.claude/projects (historical transcripts).
// listSessionsIncludingArchived is the resumable-session list: every row the
// daemon still holds, stopped and archived ones included. The plain
// listSessions above omits archived rows, which is right for the live fleet and
// wrong for a history list — an archived session is exactly the kind a user
// resumes. TWIN: the `?include_archived=true` query in recentSessions.ts.
func (c *claudemonClient) listSessionsIncludingArchived(ctx context.Context) (json.RawMessage, error) {
	return c.getRaw(ctx, "/sessions?include_archived=true")
}

func (c *claudemonClient) transcript(ctx context.Context, id, cwd string) (json.RawMessage, error) {
	path := "/sessions/" + id + "/transcript"
	if cwd != "" {
		path += "?cwd=" + url.QueryEscape(cwd)
	}
	return c.getRaw(ctx, path)
}

func (c *claudemonClient) conversation(ctx context.Context, id string, sinceSeq *int) (json.RawMessage, error) {
	path := "/sessions/" + id + "/conversation"
	if sinceSeq != nil {
		path += fmt.Sprintf("?since=%d", *sinceSeq)
	}
	return c.getRaw(ctx, path)
}

func (c *claudemonClient) subagentConversation(ctx context.Context, id, agentID string) (json.RawMessage, error) {
	path := "/sessions/" + url.PathEscape(id) + "/subagents/" + url.PathEscape(agentID) + "/conversation"
	return c.getRaw(ctx, path)
}

// spawnReq is the /sessions/spawn payload (services/claudemon/src/daemon/spawn.rs).
type spawnReq struct {
	Argv      []string          `json:"argv"`
	Cwd       string            `json:"cwd"`
	Cols      int               `json:"cols,omitempty"`
	Rows      int               `json:"rows,omitempty"`
	Env       map[string]string `json:"env,omitempty"`
	SessionID string            `json:"session_id,omitempty"`
	// FirstMessage is the agent's first prompt, queued by the daemon INSIDE its
	// spawn handler (before the 200) rather than posted by us afterwards. The
	// two-call form races: the daemon hands back an addressable id for a
	// session whose child is not up yet.
	FirstMessage string `json:"first_message,omitempty"`
}

// spawn launches a command in a PTY inside claudemon and returns the session id
// claudemon assigned (the one we pinned, when we pin one) plus whether it
// confirmed queuing the first message (`first_message_queued` — false from a
// daemon that predates the field, which is exactly what the caller needs to
// know before reporting a dispatch as delivered).
func (c *claudemonClient) spawn(ctx context.Context, req spawnReq) (string, bool, error) {
	var resp struct {
		SessionID      string `json:"session_id"`
		FirstMsgQueued bool   `json:"first_message_queued"`
	}
	if err := c.postJSON(ctx, "/sessions/spawn", req, &resp); err != nil {
		return "", false, err
	}
	if resp.SessionID == "" {
		return "", false, fmt.Errorf("spawn response missing session_id")
	}
	return resp.SessionID, resp.FirstMsgQueued, nil
}

// spawnManagedReq is the /sessions/spawn-managed payload (SpawnManagedPayload
// in services/claudemon/src/daemon/spawn.rs) — snake_case multi-word fields;
// the resume id rides the `resume` field. Note there is no `transport` key for
// claude: spawn-managed claude IS the stream adapter, so only codex's headless
// 'stream' transport is spelled out on the wire.
type spawnManagedReq struct {
	Provider string `json:"provider"`
	Cwd      string `json:"cwd"`
	Model    string `json:"model,omitempty"`
	// Reasoning-effort level (codex `model_reasoning_effort`); others ignore it.
	Effort string `json:"effort,omitempty"`
	// Resolved launcher binary (falls back to the provider name daemon-side).
	Bin string `json:"bin,omitempty"`
	// YOLO / skip approvals. False from the brain for every bus caller except a
	// hub-stamped full-access spawn (yoloGranted — see the security clamp in the
	// spawn handler): the clamp zeroes skipPermissions unless the hub verified
	// the caller's token grant.
	Yolo bool `json:"yolo"`
	// Managed providers register the Workspacer MCP facade through their own
	// config path. Claude stream receives the facade through ExtraArgs instead.
	MCP string `json:"mcp,omitempty"`
	// Role instructions prepended by claudemon to the first managed-provider turn.
	Instructions string `json:"instructions,omitempty"`
	// Codex only: "stream" runs headless (GUI-only, no native TUI PTY).
	Transport string `json:"transport,omitempty"`
	// Claude only: full permission mode (`--permission-mode`).
	PermissionMode string `json:"permission_mode,omitempty"`
	// Claude/codex: resume this prior session instead of starting fresh.
	Resume string `json:"resume,omitempty"`
	// Claude only: extra argv appended verbatim (profile extras).
	ExtraArgs []string `json:"extra_args,omitempty"`
	// Claude only: env merged over the daemon's (a profile's CLAUDE_CONFIG_DIR).
	Env map[string]string `json:"env,omitempty"`
	// Caller-pinned session id, so every client converges on one card.
	SessionID string `json:"session_id,omitempty"`
	// FirstMessage is the agent's first prompt. DISTINCT from the payload's
	// `instructions` (which the brain uses for role/terminal contracts): it is a
	// passive prefix the adapter prepends to whatever prompt arrives first and
	// never starts a turn on its own, so a dispatch put there would wait
	// forever for the prompt it is.
	FirstMessage string `json:"first_message,omitempty"`
}

// spawnManaged launches an adapter-driven (Tier-2) session — Codex/OpenCode/Pi,
// or Claude on the headless stream-json transport — and returns the session id.
func (c *claudemonClient) spawnManaged(ctx context.Context, req spawnManagedReq) (string, bool, error) {
	var resp struct {
		SessionID      string `json:"session_id"`
		FirstMsgQueued bool   `json:"first_message_queued"`
	}
	if err := c.postJSON(ctx, "/sessions/spawn-managed", req, &resp); err != nil {
		return "", false, err
	}
	if resp.SessionID == "" {
		return "", false, fmt.Errorf("spawn-managed response missing session_id")
	}
	return resp.SessionID, resp.FirstMsgQueued, nil
}

func (c *claudemonClient) getSession(ctx context.Context, id string) (json.RawMessage, error) {
	return c.getRaw(ctx, "/sessions/"+id)
}

// providerModels live-queries a managed provider's model catalog via
// GET /providers/:provider/models (services/claudemon handle_provider_models),
// which spawns the provider's own CLI in cwd. bin is the resolved launcher path
// (honoring the user's config override); cwd scopes the query. Returns the raw
// { "models": [...] } body for the handler to unwrap.
func (c *claudemonClient) providerModels(ctx context.Context, provider, cwd, bin string) (json.RawMessage, error) {
	q := url.Values{}
	if cwd != "" {
		q.Set("cwd", cwd)
	}
	if bin != "" {
		q.Set("bin", bin)
	}
	path := "/providers/" + provider + "/models"
	if enc := q.Encode(); enc != "" {
		path += "?" + enc
	}
	return c.getRaw(ctx, path)
}

// streamSSE follows an SSE endpoint, calling emit per frame. Uses a no-timeout
// client — SSE is long-lived (the shared client's 30s timeout would kill it).
func (c *claudemonClient) streamSSE(ctx context.Context, path string, emit func(name string, data []byte)) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, c.base+path, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Accept", "text/event-stream")
	resp, err := (&http.Client{}).Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	// A non-2xx answer is NOT a stream. Handing the error body to parseSSE finds
	// no frames and returns nil — "the stream ended normally" — so a PERMANENT
	// failure (claudemon's host_guard 403, a 404 after a route rename, any 5xx)
	// looked identical to a clean disconnect and reconnected forever with the
	// live-agent plane empty and nothing said anywhere. Both sibling copies
	// already check: internal/claudemon/bridge.go and the desktop's
	// lib/sseConsumer.ts (`if (!res.ok) throw new Error(HTTP ${res.status})`).
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("claudemon %s: HTTP %d: %s", path, resp.StatusCode, strings.TrimSpace(string(body)))
	}
	return parseSSE(ctx, resp.Body, emit)
}

func (c *claudemonClient) streamEvents(ctx context.Context, emit func(name string, data []byte)) error {
	return c.streamSSE(ctx, "/events", emit)
}

func (c *claudemonClient) streamStatusLines(ctx context.Context, emit func(name string, data []byte)) error {
	return c.streamSSE(ctx, "/statusline/stream", emit)
}

// submitMessage posts a prompt through claudemon's settle+verify /message
// pipeline (sent at the prompt, queued mid-turn / behind dialogs). A 409 now
// only means the session has ended — reported as ok=false (not an error) so
// the caller can surface it. Other HTTP failures are errors.
func (c *claudemonClient) submitMessage(ctx context.Context, id, text string) (ok bool, err error) {
	buf, err := json.Marshal(map[string]any{"text": text})
	if err != nil {
		return false, err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.base+"/sessions/"+id+"/message", bytes.NewReader(buf))
	if err != nil {
		return false, err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return false, err
	}
	defer resp.Body.Close()
	body, _ := io.ReadAll(resp.Body)
	switch {
	case resp.StatusCode < 300:
		return true, nil
	case resp.StatusCode == http.StatusConflict:
		return false, nil // not accepting input — caller falls back to the PTY
	default:
		return false, fmt.Errorf("claudemon POST /sessions/%s/message: %s: %s", id, resp.Status, string(body))
	}
}

// input writes raw text into the session's PTY (verbatim, no newline munging).
// This is the write-side counterpart of the byte stream — how answers and
// message fallbacks are typed in, mirroring claudemonSessionClient.input.
func (c *claudemonClient) input(ctx context.Context, id, text string) error {
	return c.postJSON(ctx, "/sessions/"+id+"/input", map[string]any{"text": text, "newline": false}, nil)
}

// inputBytes writes raw base64-encoded bytes into the PTY — for terminal
// keystrokes (arrows, Ctrl-C, Esc) that text encoding would mangle.
func (c *claudemonClient) inputBytes(ctx context.Context, id, b64 string) error {
	return c.postJSON(ctx, "/sessions/"+id+"/input", map[string]any{"bytes_b64": b64, "newline": false}, nil)
}

// gate toggles the approval gate (claudemon holds every permission prompt for an
// explicit decision when on).
func (c *claudemonClient) gate(ctx context.Context, id string, on bool) (json.RawMessage, error) {
	var out json.RawMessage
	err := c.postRaw(ctx, "/sessions/"+id+"/gate", map[string]any{"on": on}, &out)
	return out, err
}

// postRaw posts JSON and captures the response body as raw JSON.
func (c *claudemonClient) postRaw(ctx context.Context, path string, body any, out *json.RawMessage) error {
	buf, err := json.Marshal(body)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, c.base+path, bytes.NewReader(buf))
	if err != nil {
		return err
	}
	req.Header.Set("Content-Type", "application/json")
	resp, err := c.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if resp.StatusCode >= 400 {
		return fmt.Errorf("claudemon POST %s: %s: %s", path, resp.Status, string(raw))
	}
	if out != nil {
		*out = json.RawMessage(raw)
	}
	return nil
}

// answer resolves a parked AskUserQuestion structurally through the adapter's
// control protocol (POST /sessions/:id/answer). This is the ONLY way to answer a
// headless stream-transport session — it has no PTY to type into — mirroring
// claudemonSessionClient.answer on the desktop.
func (c *claudemonClient) answer(ctx context.Context, id string, option *int, text *string, answers []string) error {
	body := map[string]any{}
	if option != nil {
		body["option"] = *option
	}
	if text != nil {
		body["text"] = *text
	}
	if answers != nil {
		body["answers"] = answers
	}
	return c.postJSON(ctx, "/sessions/"+id+"/answer", body, nil)
}

// sessionTransport reports a session's transport ("pty" | "stream") from its
// claudemon snapshot. Any lookup/parse failure yields "" so the caller stays on
// the default PTY keystroke path.
func (c *claudemonClient) sessionTransport(ctx context.Context, id string) string {
	raw, err := c.getSession(ctx, id)
	if err != nil {
		return ""
	}
	var s struct {
		Transport string `json:"transport"`
	}
	if err := json.Unmarshal(raw, &s); err != nil {
		return ""
	}
	return s.Transport
}

func (c *claudemonClient) approve(ctx context.Context, id, decision, reason string) error {
	body := map[string]any{"decision": decision}
	if reason != "" {
		body["reason"] = reason
	}
	return c.postJSON(ctx, "/sessions/"+id+"/approve", body, nil)
}

func (c *claudemonClient) resize(ctx context.Context, id string, cols, rows int) error {
	return c.postJSON(ctx, "/sessions/"+id+"/resize", map[string]any{"cols": cols, "rows": rows}, nil)
}

func (c *claudemonClient) signal(ctx context.Context, id, signal string) error {
	return c.postJSON(ctx, "/sessions/"+id+"/signal", map[string]any{"signal": signal}, nil)
}

// permissionModeResult is claudemon's answer to a live permission-mode switch.
// `mode` is the mode the DAEMON confirmed (it drives and verifies the switch —
// claude by cycling shift+tab against the screen, codex by the adapter's
// approval flag), which is not necessarily the one that was asked for.
type permissionModeResult struct {
	Mode  string `json:"mode"`
	Error string `json:"error"`
}

// setPermissionMode live-switches a running session's permission mode.
// TWIN: claudemonSessionClient.setPermissionMode.
//
// A non-2xx is an ERROR here, not an ok:false — the CALLER (registry
// .setPermissionMode) turns it into the `{ok:false, error}` envelope the
// desktop's handler answers with, so this client stays the thin HTTP twin.
func (c *claudemonClient) setPermissionMode(ctx context.Context, id, mode string) (permissionModeResult, error) {
	var out json.RawMessage
	if err := c.postRaw(ctx, "/sessions/"+id+"/permission-mode", map[string]any{"mode": mode}, &out); err != nil {
		return permissionModeResult{}, err
	}
	var res permissionModeResult
	_ = json.Unmarshal(out, &res)
	return res, nil
}

// setModel live-switches a managed session's model and/or reasoning effort
// (codex applies it to the running thread via thread/settings/update). Empty
// fields are OMITTED rather than sent as "": the daemon reads a present-but-
// empty model as a request to switch to a model with no name.
// TWIN: claudemonSessionClient.setModel.
func (c *claudemonClient) setModel(ctx context.Context, id, model, effort string) error {
	body := map[string]any{}
	if model != "" {
		body["model"] = model
	}
	if effort != "" {
		body["effort"] = effort
	}
	return c.postJSON(ctx, "/sessions/"+id+"/model", body, nil)
}

// handoffResult is claudemon's cross-provider handoff brief: the markdown and
// the path it was persisted at (~/.workspacer/handoffs/). The daemon composes
// both — no caller string reaches either.
type handoffResult struct {
	Markdown string `json:"markdown"`
	Path     string `json:"path"`
}

// handoff builds and persists a session's handoff brief.
// TWIN: claudemonSessionClient.handoffBrief.
func (c *claudemonClient) handoff(ctx context.Context, id string) (handoffResult, error) {
	var out json.RawMessage
	if err := c.postRaw(ctx, "/sessions/"+id+"/handoff", map[string]any{}, &out); err != nil {
		return handoffResult{}, err
	}
	var res handoffResult
	_ = json.Unmarshal(out, &res)
	return res, nil
}

// closeSession asks the daemon to terminate a session's child process. TWIN:
// claudemonSessionClient.close, whose only escalation beyond stopping its own
// viewers is exactly this SIGTERM — the same one claude.signal already offers.
// The session row stays in the daemon as a resumable Stopped row; that is
// intended, and it is why agents.close ALSO forgets the row locally.
func (c *claudemonClient) closeSession(ctx context.Context, id string) error {
	return c.signal(ctx, id, "SIGTERM")
}
