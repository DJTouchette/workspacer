package main

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/federation"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestSummaryCompatibilityDoesNotSwallowDenial(t *testing.T) {
	for _, message := range []string{"no provider for agents.summarizeStatus", "hub:peer: no provider for agents.summarizeStatus", "unknown method: hub:peer/agents.summarizeStatus"} {
		if !summaryUnavailableError(message, "hub:peer/agents.summarizeStatus") {
			t.Errorf("not recognized: %s", message)
		}
	}
	for _, message := range []string{"permission denied: no provider for agents.summarizeStatus", "hub:peer: permission denied", "scope may not call agents.summarizeStatus"} {
		if summaryUnavailableError(message, "hub:peer/agents.summarizeStatus") {
			t.Errorf("swallowed denial: %s", message)
		}
	}
	if !validSummaryResponse(unavailableSummary("desktop-summary-unavailable")) {
		t.Fatal("own envelope invalid")
	}
	if validSummaryResponse(`{"contract":"agent-status-summary/v1","status":"ok","transcript":"SECRET"}`) {
		t.Fatal("accepted transcript")
	}
}
func TestSummaryToolIsViewTier(t *testing.T) {
	for _, scope := range []authtoken.Scope{authtoken.ScopeView, authtoken.ScopeTriage, authtoken.ScopeOperator} {
		if !listToolsFor(t, scope)["summarize_agent_status"] {
			t.Fatalf("missing in %s", scope)
		}
	}
}
func TestSummaryFederationOwnerOldPeerAndDenial(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	peerBus := bus.NewServer(broker.New())
	var calls atomic.Int32
	peerBus.RegisterLocal(statusSummaryMethod, func(params json.RawMessage) (any, error) {
		calls.Add(1)
		var in map[string]any
		_ = json.Unmarshal(params, &in)
		if _, ok := in["hub"]; ok {
			t.Error("hub leaked to provider")
		}
		if in["sessionId"] == "denied" {
			return nil, errors.New("permission denied: no provider for agents.summarizeStatus")
		}
		var result map[string]any
		_ = json.Unmarshal([]byte(unavailableSummary("peer-only")), &result)
		return result, nil
	})
	peerSrv := httptest.NewServer(peerBus.Handler())
	defer peerSrv.Close()
	peerURL := strings.Replace(peerSrv.URL, "http", "ws", 1) + "/bus"
	oldSrv := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer oldSrv.Close()
	oldURL := strings.Replace(oldSrv.URL, "http", "ws", 1) + "/bus"
	localURL, fed := newFederatedLocalHub(t, ctx, []federation.Peer{{Name: "peer", URL: peerURL}, {Name: "old", URL: oldURL}})
	// A matching session id exists locally; a remote call must never choose it.
	machineProvider(t, ctx, localURL, []string{statusSummaryMethod}, func(_ string, _ json.RawMessage) any {
		var result map[string]any
		_ = json.Unmarshal([]byte(unavailableSummary("LOCAL-LOOKALIKE")), &result)
		return result
	})
	waitPeerConnected(t, fed, "peer")
	waitPeerConnected(t, fed, "old")
	cs := connectFacade(t, ctx, localURL, authtoken.ScopeView)
	// Wait for the facade bus connection using the same eventual pattern as siblings.
	var text string
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		res := callTool(t, ctx, cs, "summarize_agent_status", map[string]any{"hub": "peer", "sessionId": "same-id"})
		text = textOf(res)
		if strings.Contains(text, "peer-only") {
			break
		}
		time.Sleep(20 * time.Millisecond)
	}
	if !strings.Contains(text, "peer-only") || strings.Contains(text, "LOCAL-LOOKALIKE") {
		t.Fatalf("wrong owner: %s", text)
	}
	res := callTool(t, ctx, cs, "summarize_agent_status", map[string]any{"hub": "old", "sessionId": "same-id"})
	if res.IsError || !strings.Contains(textOf(res), "desktop-summary-unavailable") {
		t.Fatalf("old peer: %s", textOf(res))
	}
	res = callTool(t, ctx, cs, "summarize_agent_status", map[string]any{"hub": "peer", "sessionId": "denied"})
	if !res.IsError || textOf(res) != "permission denied" {
		t.Fatalf("denial swallowed: %s", textOf(res))
	}
	if calls.Load() < 2 {
		t.Fatal("peer not called")
	}
}
