package main

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"
)

// Private callback from the owner-only intent service. Reuse the existing
// daemon message transport and route peers by the persisted hub identity.
func (r *registry) sendIntentDirection(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Hub       string `json:"hub"`
		Text      string `json:"text"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if err := validateSessionConfigName(p.SessionID); err != nil {
		return nil, err
	}
	if strings.TrimSpace(p.Text) == "" || len(p.Text) > 256*1024 {
		return nil, fmt.Errorf("invalid intent direction")
	}
	if p.Hub != "" {
		if err := validateSessionConfigName(p.Hub); err != nil {
			return nil, err
		}
		if r.callHub == nil {
			return nil, fmt.Errorf("peer messaging unavailable")
		}
		params := map[string]string{"sessionId": p.SessionID, "text": p.Text}
		result, err := r.callHub(ctx, "hub:"+p.Hub+"/agents.sendMessage", params)
		if err != nil {
			return nil, err
		} // A peer exception is not proof of non-delivery.
		var receipt struct {
			OK bool `json:"ok"`
		}
		if json.Unmarshal(result, &receipt) != nil || !receipt.OK {
			return jsonResult(map[string]string{"status": "unknown", "detail": "The peer did not return a confirmed message acknowledgment."})
		}
		return jsonResult(map[string]string{"status": "accepted", "detail": "Accepted by the peer messaging service; the message may be queued."})
	}
	status := r.cm.deliverCaptured(ctx, p.SessionID, p.Text)
	detail := "Delivery acknowledgment is unknown; inspect the agent conversation before taking further action."
	if status == "rejected" {
		status = "failed"
		detail = "The daemon rejected the message."
	}
	if status == "accepted" {
		detail = "Accepted by the local messaging service; the message may be queued."
	}
	return jsonResult(map[string]string{"status": status, "detail": detail})
}

func (r *registry) interruptIntentExecution(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
		Hub       string `json:"hub"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if err := validateSessionConfigName(p.SessionID); err != nil {
		return nil, err
	}
	if p.Hub != "" {
		if err := validateSessionConfigName(p.Hub); err != nil {
			return nil, err
		}
		if r.callHub == nil {
			return nil, fmt.Errorf("peer signal service unavailable")
		}
		raw, err := r.callHub(ctx, "hub:"+p.Hub+"/claude.signal", map[string]string{"sessionId": p.SessionID, "signal": "SIGINT"})
		if err != nil {
			return nil, err
		}
		var receipt struct {
			OK bool `json:"ok"`
		}
		if json.Unmarshal(raw, &receipt) != nil || !receipt.OK {
			return jsonResult(map[string]string{"status": "unknown", "detail": "The peer did not confirm interrupt acceptance."})
		}
	} else if err := r.cm.signal(ctx, p.SessionID, "SIGINT"); err != nil {
		return nil, err
	}
	return jsonResult(map[string]string{"status": "accepted", "detail": "The signal service accepted the interrupt request. Inspect the session; background work may continue and earlier actions are not undone."})
}

// Only the owner child can activate an intent. No caller-supplied grant fields
// cross this boundary; ordinary spawn permission policy still applies.
func (r *registry) spawnIntentManager(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		Cwd      string `json:"cwd"`
		Label    string `json:"label"`
		Message  string `json:"message"`
		Provider string `json:"provider"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if strings.TrimSpace(p.Cwd) == "" || strings.TrimSpace(p.Message) == "" || len(p.Message) > 256*1024 {
		return nil, fmt.Errorf("invalid intent launch")
	}
	params, err := json.Marshal(map[string]any{"cwd": p.Cwd, "label": p.Label, "message": p.Message, "provider": p.Provider, "manager": true, "toolScope": "operator", "transport": "stream"})
	if err != nil {
		return nil, err
	}
	return r.spawn(ctx, params)
}
