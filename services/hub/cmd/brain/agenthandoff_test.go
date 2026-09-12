package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestAgentAuthoredHandoffAndFallback(t *testing.T) {
	setHome(t, t.TempDir())
	priorWait, priorPoll := agentBriefWait, agentBriefPoll
	agentBriefWait = 30 * time.Millisecond
	agentBriefPoll = time.Millisecond
	t.Cleanup(func() { agentBriefWait = priorWait; agentBriefPoll = priorPoll })
	writeBrief := true
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if strings.HasSuffix(req.URL.Path, "/handoff") {
			_, _ = w.Write([]byte(`{"path":"fallback.md","markdown":"mechanical"}`))
			return
		}
		var p struct {
			Text string `json:"text"`
		}
		_ = json.NewDecoder(req.Body).Decode(&p)
		if writeBrief {
			target := strings.Split(strings.TrimPrefix(p.Text, "Stop what you're doing and write a handoff brief to "), " — another AI")[0]
			if filepath.Dir(target) != filepath.Join(homeDir(), ".workspacer", "handoffs") {
				t.Error("brief escaped host directory")
			}
			_ = os.WriteFile(target, []byte("# Actual authored handoff\n"), 0600)
		}
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	r := newRegistry(newClaudemonClient(srv.URL))
	out, err := r.handle(context.Background(), "claude.handoffAgentBrief", json.RawMessage(`{"sessionId":"s1"}`))
	if err != nil || !strings.Contains(string(out), `"ok":true`) || strings.Contains(string(out), `"fallback":true`) {
		t.Fatalf("authored handoff: %s / %v", out, err)
	}
	writeBrief = false
	out, err = r.handle(context.Background(), "claude.handoffAgentBrief", json.RawMessage(`{"sessionId":"s1"}`))
	if err != nil || !strings.Contains(string(out), `"fallback":true`) {
		t.Fatalf("fallback: %s / %v", out, err)
	}
	if _, err = r.handle(context.Background(), "claude.handoffAgentBrief", json.RawMessage(`{"sessionId":"../../outside"}`)); err == nil {
		t.Fatal("unsafe session id accepted")
	}
}
