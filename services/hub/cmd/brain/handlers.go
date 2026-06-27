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
	"os"
)

// registry holds the dependencies the handlers close over and dispatches calls
// by method name.
type registry struct {
	cm *claudemonClient
}

func newRegistry(cm *claudemonClient) *registry {
	return &registry{cm: cm}
}

// methods is the set of capabilities this provider registers on the bus.
func (r *registry) methods() []string {
	return []string{
		"agents.list",
		"agents.spawn",
		"agents.sendMessage",
		"claude.approve",
		"claude.answer",
		"claude.signal",
		"sessions.transcript",
		"sessions.conversation",
		"claude.profiles.list",
	}
}

// handle dispatches one capability call.
func (r *registry) handle(ctx context.Context, method string, params json.RawMessage) (json.RawMessage, error) {
	switch method {
	case "agents.list":
		return r.cm.listSessions(ctx)
	case "agents.spawn":
		return r.spawn(ctx, params)
	case "agents.sendMessage":
		return r.sendMessage(ctx, params)
	case "claude.approve":
		return r.approve(ctx, params)
	case "claude.answer":
		return r.answer(ctx, params)
	case "claude.signal":
		return r.signal(ctx, params)
	case "sessions.transcript":
		return r.transcript(ctx, params)
	case "sessions.conversation":
		return r.conversation(ctx, params)
	case "claude.profiles.list":
		return jsonResult(loadProfiles())
	default:
		return nil, fmt.Errorf("unknown method %q", method)
	}
}

// ── param shapes (match the MCP facade / app capability inputs) ─────────────

type spawnParams struct {
	Cwd             string `json:"cwd"`
	Model           string `json:"model"`
	ProfileID       string `json:"profileId"`
	SkipPermissions bool   `json:"skipPermissions"`
	ResumeSessionID string `json:"resumeSessionId"`
	Cols            int    `json:"cols"`
	Rows            int    `json:"rows"`
}

type sessionParam struct {
	SessionID string `json:"sessionId"`
}

func (r *registry) spawn(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p spawnParams
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}

	cwd := normalizeCwd(p.Cwd)
	if cwd == "" {
		home, _ := os.UserHomeDir()
		cwd = home
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

	cols, rows := p.Cols, p.Rows
	if cols == 0 {
		cols = 120
	}
	if rows == 0 {
		rows = 32
	}

	id, err := r.cm.spawn(ctx, spawnReq{
		Argv:      buildArgv(prof, p.Model, p.SkipPermissions, sessionID, resume),
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
	if err := r.cm.message(ctx, p.SessionID, p.Text); err != nil {
		return nil, err
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

func (r *registry) answer(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string  `json:"sessionId"`
		Option    *int    `json:"option"`
		Text      *string `json:"text"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("claude.answer requires { sessionId }")
	}
	switch {
	case p.Option != nil:
		if err := r.cm.answerOption(ctx, p.SessionID, *p.Option); err != nil {
			return nil, err
		}
	case p.Text != nil:
		if err := r.cm.answerText(ctx, p.SessionID, *p.Text); err != nil {
			return nil, err
		}
	default:
		return nil, fmt.Errorf("claude.answer requires an option or text")
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
	var p sessionParam
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("sessions.transcript requires { sessionId }")
	}
	return r.cm.transcript(ctx, p.SessionID)
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
