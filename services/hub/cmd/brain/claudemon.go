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

func (c *claudemonClient) transcript(ctx context.Context, id string) (json.RawMessage, error) {
	return c.getRaw(ctx, "/sessions/"+id+"/transcript")
}

func (c *claudemonClient) conversation(ctx context.Context, id string, sinceSeq *int) (json.RawMessage, error) {
	path := "/sessions/" + id + "/conversation"
	if sinceSeq != nil {
		path += fmt.Sprintf("?since=%d", *sinceSeq)
	}
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
}

// spawn launches a command in a PTY inside claudemon and returns the session id
// claudemon assigned (the one we pinned, when we pin one).
func (c *claudemonClient) spawn(ctx context.Context, req spawnReq) (string, error) {
	var resp struct {
		SessionID string `json:"session_id"`
	}
	if err := c.postJSON(ctx, "/sessions/spawn", req, &resp); err != nil {
		return "", err
	}
	if resp.SessionID == "" {
		return "", fmt.Errorf("spawn response missing session_id")
	}
	return resp.SessionID, nil
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
