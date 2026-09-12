package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"time"
)

var agentBriefWait = 150 * time.Second
var agentBriefPoll = time.Second
var handoffSessionID = regexp.MustCompile(`^[A-Za-z0-9_-]{1,128}$`)

func agentBriefInstruction(path string) string {
	return "Stop what you're doing and write a handoff brief to " + path + " — another AI coding agent is about to take over this session and will read that file first. Create the file (markdown) with:\n" +
		"1. The goal of this session, in one paragraph.\n2. State of the work: what's done and verified, what's in progress, what hasn't been started.\n3. Key files touched and why.\n4. Decisions and constraints your successor must respect (including approaches tried and rejected, and why).\n5. Gotchas or surprises you hit.\n6. The exact next step you would take.\nWrite only that file, then reply \"Handoff brief written.\" — do not continue any other work."
}

func (r *registry) agentHandoffBrief(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
	}
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if !handoffSessionID.MatchString(p.SessionID) {
		return nil, fmt.Errorf("invalid sessionId")
	}
	fallback := func(reason string) (json.RawMessage, error) {
		brief, err := r.cm.handoff(ctx, p.SessionID)
		if err != nil {
			return jsonResult(map[string]any{"ok": false, "error": reason + "; mechanical fallback also failed: " + err.Error()})
		}
		return jsonResult(map[string]any{"ok": brief.Path != "", "path": brief.Path, "fallback": true, "error": reason})
	}
	dir := filepath.Join(homeDir(), ".workspacer", "handoffs")
	if err := os.MkdirAll(dir, 0700); err != nil {
		return nil, err
	}
	nonce, err := newSessionID()
	if err != nil {
		return nil, err
	}
	target := filepath.Join(dir, time.Now().UTC().Format("20060102-150405")+"-"+nonce[:8]+"-agent.md")
	sent, err := r.cm.submitMessage(ctx, p.SessionID, agentBriefInstruction(target))
	if err != nil || !sent {
		return fallback("Source agent could not accept the brief request")
	}
	timer := time.NewTimer(agentBriefWait)
	defer timer.Stop()
	tick := time.NewTicker(agentBriefPoll)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-timer.C:
			return fallback("Source agent did not write the brief before the deadline")
		case <-tick.C:
			info, err := os.Lstat(target)
			if err == nil && info.Mode().IsRegular() && info.Size() > 0 {
				return jsonResult(map[string]any{"ok": true, "path": target})
			}
		}
	}
}
