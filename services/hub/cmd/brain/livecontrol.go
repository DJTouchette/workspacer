package main

// LIVE CONTROL of an agent that is ALREADY RUNNING: claude.setPermissionMode /
// claude.setEffort / claude.setModel, plus claude.handoffBrief.
//
// TWIN: the control block of apps/desktop/src/main/services/hubCapabilities.ts
// (lines around claude.setPermissionMode / setEffort / setModel /
// handoffBrief), which delegates to claudemonSessionClient and
// services/liveEffort.ts. The desktop registers all four inside Electron main,
// so a node running `brain --hub <ws>` answered every one with "no provider":
// the composer's mode pill, model switcher and effort control were inert on
// /app, and /m surfaced the failure loudly while /app swallowed it into a
// console.warn. Same methods, same wire shapes, second provider.
//
// THE ESCALATION CLAMP IS THE POINT OF THE PORT.
//
// agents.spawn already refuses to let a bus caller start an agent with
// approvals off. claude.setPermissionMode reaches an agent that is already
// running — including one the LOCAL user started in ask mode — and it does no
// ownership check on the sessionId. Without the same clamp, "spawn clamped +
// setPermissionMode unclamped" is one extra call to bypassPermissions followed
// by agents.sendMessage: the exact escalation the spawn clamp exists to refuse,
// reached through a second door. The desktop closes it with
// assertNoPermissionBypass (lib/permissionBypass.ts); this closes it with
// permissionmode.go's isPermissionEscalation, which is the SAME allowlist
// (pinned to the desktop by TestPermissionModeAllowlistMatchesTheDesktop).
//
// Direction matters and is deliberate: DE-ESCALATING and neutral modes stay
// open, because tightening is not an escalation and a remote operator must be
// able to put a runaway worker back into ask mode. That is why the refusal is
// asymmetric rather than "no mode changes from the bus".

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/modelselection"
)

// liveControlResult is the shared `{ok, error?}` envelope the three switches
// answer with. A provider that cannot do the switch LIVE is ok:false with a
// reason — not a transport error — so the client can offer the restart path.
// The desktop's three handlers return exactly these shapes.
type liveControlResult struct {
	OK                 bool                      `json:"ok"`
	Mode               string                    `json:"mode,omitempty"`
	Effort             string                    `json:"effort,omitempty"`
	Error              string                    `json:"error,omitempty"`
	RequestedSelection *modelselection.Selection `json:"requestedSelection,omitempty"`
}

// setPermissionMode live-switches an already-running session's permission mode.
//
// REFUSED BEFORE IT TRAVELS: an escalating mode never reaches claudemon at all,
// and the CHECKED string is the one sent (there is no second read of the
// caller's field between the check and the send).
func (r *registry) setPermissionMode(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Mode      string `json:"mode"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" || p.Mode == "" {
		return nil, fmt.Errorf("claude.setPermissionMode requires { sessionId, mode }")
	}
	if isPermissionEscalation(p.Mode) {
		// The same sentence shape the spawn clamp uses: name the refusal, not a
		// generic failure, so a caller cannot read it as "the daemon was busy".
		return nil, fmt.Errorf("claude.setPermissionMode: refusing to set permission mode %q from the bus — "+
			"a mode that stops the host asking for approvals has to be chosen locally", p.Mode)
	}
	res, err := r.cm.setPermissionMode(ctx, p.SessionID, p.Mode)
	if err != nil {
		return jsonResult(liveControlResult{OK: false, Error: err.Error()})
	}
	mode := res.Mode
	if mode == "" {
		mode = p.Mode
	}
	// The DAEMON-CONFIRMED mode is what is recorded, not the requested one:
	// claudemon drives and verifies the switch (claude by cycling shift+tab
	// against the screen), so it can land somewhere other than where it was
	// aimed, and a pill showing the request rather than the result is the same
	// lie in the other direction.
	r.noteLiveControl(p.SessionID, mode, "", "")
	return jsonResult(liveControlResult{OK: true, Mode: mode})
}

// setModel live-switches a managed session's model and/or reasoning effort.
// Claude PTY sessions do not use this endpoint (they switch via the `/model`
// slash command on the message path). Managed Claude stream and other capable
// managed providers use the daemon's structural switch endpoint.
func (r *registry) setModel(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID     string  `json:"sessionId"`
		Model         string  `json:"model"`
		ModelIdentity string  `json:"modelIdentity"`
		ContextWindow *uint64 `json:"contextWindow"`
		Effort        string  `json:"effort"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" || (p.Model == "" && p.ModelIdentity == "" && p.Effort == "") {
		return nil, fmt.Errorf("claude.setModel requires { sessionId, model and/or effort }")
	}
	provider := r.sessionProvider(ctx, p.SessionID)
	resolved, err := modelselection.ResolveInput(
		provider,
		p.Model,
		p.ModelIdentity,
		p.ContextWindow,
	)
	if err != nil {
		return jsonResult(liveControlResult{OK: false, Error: modelselection.ErrorCode(err)})
	}
	model, identity := p.Model, p.ModelIdentity
	window := p.ContextWindow
	if resolved != nil {
		model = resolved.LegacyModel
		identity = resolved.Selection.Model
		window = resolved.Selection.ContextWindow
	}
	if err := r.cm.setModel(ctx, p.SessionID, model, p.Effort, identity, window); err != nil {
		return jsonResult(liveControlResult{OK: false, Error: err.Error()})
	}
	// Noted eagerly so the context-window figure follows an `opus[1m]` switch
	// immediately — the provider confirms on its own status line, which
	// supersedes this.
	r.noteLiveControl(p.SessionID, "", model, p.Effort)
	var selection *modelselection.Selection
	if resolved != nil {
		selection = &resolved.Selection
	}
	return jsonResult(liveControlResult{OK: true, RequestedSelection: selection})
}

// setEffort live-switches reasoning effort. TWIN: services/liveEffort.ts, and
// this is the one switch that BRANCHES ON PROVIDER, which is exactly why the
// desktop factored it into a single shared body rather than inlining it twice:
//
//   - claude (both transports): the `/effort <level>` slash command through the
//     normal queued message path. There is no set_effort in the stream control
//     protocol, so the message path is the mechanism on stream too.
//   - managed (codex): the daemon's /sessions/:id/model with effort only, which
//     applies thread/settings/update to the running thread.
//
// A session whose provider cannot be resolved is treated as claude, matching
// liveEffort.ts's `?? 'claude'` default.
func (r *registry) setEffort(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Effort    string `json:"effort"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	level := strings.TrimSpace(p.Effort)
	if p.SessionID == "" || level == "" {
		return jsonResult(liveControlResult{OK: false, Error: "requires a session and an effort level"})
	}
	if r.sessionProvider(ctx, p.SessionID) == "claude" {
		ok, err := r.cm.submitMessage(ctx, p.SessionID, "/effort "+level)
		if err != nil {
			return jsonResult(liveControlResult{OK: false, Error: err.Error()})
		}
		if !ok {
			return jsonResult(liveControlResult{OK: false, Error: "this session can't take input right now (ended)"})
		}
		// For a CLAUDE session this note is the pill's ONLY truth: effective
		// effort appears in no hook, status line or init frame.
		r.noteLiveControl(p.SessionID, "", "", level)
		return jsonResult(liveControlResult{OK: true, Effort: level})
	}
	if err := r.cm.setModel(ctx, p.SessionID, "", level, "", nil); err != nil {
		return jsonResult(liveControlResult{OK: false, Error: err.Error()})
	}
	// codex confirms with its own thread/settings/updated on the status line,
	// which supersedes this.
	r.noteLiveControl(p.SessionID, "", "", level)
	return jsonResult(liveControlResult{OK: true, Effort: level})
}

// noteLiveControl records a confirmed switch in the spawn metadata AND restamps
// the live row, so the very next agents.list / sessions.snapshot carries it
// rather than waiting for claudemon's next event. A brain without a metadata
// store (catalog scope) simply does not record — the desktop owns the live view
// in that configuration and notes it itself.
func (r *registry) noteLiveControl(sessionID, permissionMode, model, effort string) {
	if r.meta == nil {
		return
	}
	r.meta.noteLiveControl(sessionID, permissionMode, model, effort)
	if r.store != nil {
		r.store.restamp(sessionID)
	}
}

// sessionProvider reads a session's provider from the live store when the brain
// holds one, else from claudemon directly. "" is reported as "claude" —
// claudemon serializes a legacy claude row with an empty provider, and
// liveEffort.ts defaults the same way.
func (r *registry) sessionProvider(ctx context.Context, id string) string {
	var raw json.RawMessage
	if r.store != nil {
		if snap, ok := r.store.get(id); ok {
			raw = snap
		}
	}
	if raw == nil {
		got, err := r.cm.getSession(ctx, id)
		if err != nil {
			return "claude"
		}
		raw = got
	}
	var s struct {
		Provider string `json:"provider"`
	}
	if json.Unmarshal(raw, &s) != nil || s.Provider == "" {
		return "claude"
	}
	return s.Provider
}

// handoffBrief builds (and persists under ~/.workspacer/handoffs/) the
// cross-provider handoff brief for a session. A pure relay: the daemon composes
// the markdown and chooses the filename, so the caller supplies a session id
// and never a path — which is exactly capspec's stated reason for leaving it
// unscoped.
//
// The AGENT-authored variant (claude.handoffAgentBrief) is deliberately NOT
// ported: it is not a relay but an orchestration that injects a write-this
// instruction into the live agent and waits for the file to appear
// (main/services/agentHandoff.ts). It stays a declared gap.
func (r *registry) handoffBrief(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("claude.handoffBrief requires { sessionId }")
	}
	res, err := r.cm.handoff(ctx, p.SessionID)
	if err != nil {
		return jsonResult(map[string]any{"ok": false, "error": err.Error()})
	}
	return jsonResult(map[string]any{"ok": true, "markdown": res.Markdown, "path": res.Path})
}
