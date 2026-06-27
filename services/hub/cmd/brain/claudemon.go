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

func (c *claudemonClient) message(ctx context.Context, id, text string) error {
	return c.postJSON(ctx, "/sessions/"+id+"/message", map[string]any{"text": text}, nil)
}

func (c *claudemonClient) approve(ctx context.Context, id, decision, reason string) error {
	body := map[string]any{"decision": decision}
	if reason != "" {
		body["reason"] = reason
	}
	return c.postJSON(ctx, "/sessions/"+id+"/approve", body, nil)
}

func (c *claudemonClient) answerOption(ctx context.Context, id string, option int) error {
	return c.postJSON(ctx, "/sessions/"+id+"/answer", map[string]any{"option": option}, nil)
}

func (c *claudemonClient) answerText(ctx context.Context, id, text string) error {
	return c.postJSON(ctx, "/sessions/"+id+"/answer", map[string]any{"text": text}, nil)
}

func (c *claudemonClient) signal(ctx context.Context, id, signal string) error {
	return c.postJSON(ctx, "/sessions/"+id+"/signal", map[string]any{"signal": signal}, nil)
}
