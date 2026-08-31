package main

// The facade is a PIPE for the daemon-owned model facts. list_agents /
// list_snapshots merge rows across hubs; they must not edit them. A row's
// `requestedSelection` / `resolvedContextWindow` has to arrive at an agent
// exactly as its home hub published it — local rows byte-for-byte, remote rows
// with nothing added but the `hub` tag that says which machine they came from.

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/federation"
)

func TestFleetToolsPassOwnerSelectionThrough(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	// The peer's row: an early 1M session whose provider status line still
	// claims 200,000 — the exact pair a client has to reconcile itself.
	peerRow := map[string]any{
		"sessionId":             "remote-1",
		"requestedSelection":    map[string]any{"model": "opus", "contextWindow": 1_000_000},
		"resolvedContextWindow": 1_000_000,
		"statusLine":            map[string]any{"contextWindowSize": 200_000},
		"usage":                 map[string]any{"contextTokens": 356_380, "contextLimit": 1_000_000},
	}
	// The local row's selection is SPARSE (identity known, window unresolved)
	// and it carries no resolved window at all — the two absences a merge is
	// most likely to paper over.
	localRow := map[string]any{
		"sessionId":          "local-1",
		"requestedSelection": map[string]any{"model": "sonnet", "contextWindow": nil},
	}

	peerSrv := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	defer peerSrv.Close()
	peerURL := strings.Replace(peerSrv.URL, "http", "ws", 1) + "/bus"
	machineProvider(t, ctx, peerURL, []string{"agents.list", "sessions.snapshots"},
		func(string, json.RawMessage) any { return []map[string]any{peerRow} })

	localURL, fed := newFederatedLocalHub(t, ctx, []federation.Peer{{Name: "work", URL: peerURL}})
	machineProvider(t, ctx, localURL, []string{"agents.list", "sessions.snapshots"},
		func(string, json.RawMessage) any { return []map[string]any{localRow} })
	waitPeerConnected(t, fed, "work")

	cs := connectFacade(t, ctx, localURL, authtoken.ScopeOperator)

	for _, tool := range []string{"list_agents", "list_snapshots"} {
		res := callTool(t, ctx, cs, tool, map[string]any{})
		if res.IsError {
			t.Fatalf("%s errored: %s", tool, textOf(res))
		}
		var rows []map[string]any
		if err := json.Unmarshal([]byte(textOf(res)), &rows); err != nil {
			t.Fatalf("%s: not a JSON array: %v (%s)", tool, err, textOf(res))
		}
		byID := map[string]map[string]any{}
		for _, r := range rows {
			id, _ := r["sessionId"].(string)
			byID[id] = r
		}

		remote, ok := byID["remote-1"]
		if !ok {
			t.Fatalf("%s: peer row missing: %s", tool, textOf(res))
		}
		if remote["hub"] != "work" {
			t.Errorf("%s: remote row hub = %v", tool, remote["hub"])
		}
		sel, _ := remote["requestedSelection"].(map[string]any)
		if sel["model"] != "opus" {
			t.Errorf("%s: remote requestedSelection = %#v", tool, remote["requestedSelection"])
		}
		if w, _ := sel["contextWindow"].(float64); w != 1_000_000 {
			t.Errorf("%s: remote requestedSelection.contextWindow = %v", tool, sel["contextWindow"])
		}
		if w, _ := remote["resolvedContextWindow"].(float64); w != 1_000_000 {
			t.Errorf("%s: remote resolvedContextWindow = %v", tool, remote["resolvedContextWindow"])
		}
		// The contradicted provider claim rides along untouched; the facade
		// does not get to pick a winner on the agent's behalf.
		sl, _ := remote["statusLine"].(map[string]any)
		if w, _ := sl["contextWindowSize"].(float64); w != 200_000 {
			t.Errorf("%s: remote statusLine.contextWindowSize = %v, want 200000", tool, sl["contextWindowSize"])
		}

		local, ok := byID["local-1"]
		if !ok {
			t.Fatalf("%s: local row missing: %s", tool, textOf(res))
		}
		if _, tagged := local["hub"]; tagged {
			t.Errorf("%s: local row must stay untagged: %v", tool, local)
		}
		localSel, _ := local["requestedSelection"].(map[string]any)
		if localSel["model"] != "sonnet" {
			t.Errorf("%s: local requestedSelection = %#v", tool, local["requestedSelection"])
		}
		if w, present := localSel["contextWindow"]; !present || w != nil {
			t.Errorf("%s: sparse selection was completed to %#v (present=%v)", tool, w, present)
		}
		if _, present := local["resolvedContextWindow"]; present {
			t.Errorf("%s: a resolved window was invented for the local row: %#v", tool, local["resolvedContextWindow"])
		}
		// Local rows are appended byte-for-byte — prove it on the raw text, not
		// just on the decoded map.
		if !strings.Contains(textOf(res), `"requestedSelection":{"contextWindow":null,"model":"sonnet"}`) {
			t.Errorf("%s: local row was re-marshaled: %s", tool, textOf(res))
		}
	}
}
