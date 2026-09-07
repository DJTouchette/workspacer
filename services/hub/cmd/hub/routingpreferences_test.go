package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/routing"
)

func TestRoutingPreferencesHandlersReachSelectAndPreviewWithoutAudit(t *testing.T) {
	path := filepath.Join(t.TempDir(), "routing.yaml")
	svc := routing.New(path, nil)
	host := bus.CallerIdentity{Trusted: true, AuthenticatedHost: true, Scope: "operator"}
	get, e := routingPreferencesGet(svc)(host, json.RawMessage(`{}`))
	if e != nil {
		t.Fatal(e)
	}
	v := get.(routing.PreferencesView)
	raw, _ := json.Marshal(map[string]any{"baseRevision": v.Revision, "patch": map[string]any{"activeProfile": "codex_only", "roles": map[string]string{"scout": "cheap"}}})
	for _, c := range []bus.CallerIdentity{{}, {Trusted: true, Scope: "operator"}, {Trusted: true, AuthenticatedHost: true, Scope: "triage"}} {
		for _, h := range []bus.LocalIdentHandler{routingPreferencesValidate(svc), routingPreferencesSave(svc), routingPreferencesReset(svc)} {
			if _, e := h(c, raw); e == nil {
				t.Fatalf("ungated caller %+v", c)
			}
		}
	}
	saved, e := routingPreferencesSave(svc)(host, raw)
	if e != nil || saved.(routing.PreferencesResult).Status != "applied" {
		t.Fatalf("save %v %v", saved, e)
	}
	usageServer := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"providers":[]}`))
	}))
	defer usageServer.Close()
	usage := newUsageWatcher(usageServer.URL)
	logPath := filepath.Join(t.TempDir(), "decisions.jsonl")
	log := routing.NewDecisionLog(logPath, routing.DefaultDecisionLogMaxBytes)
	events := 0
	selectHandler := routingSelect(svc, usage, nil, func(event.Envelope) { events++ }, log)
	selected, e := selectHandler(host, json.RawMessage(`{"role":"scout"}`))
	if e != nil {
		t.Fatal(e)
	}
	d := selected.(routing.Decision)
	if d.Model != "gpt-5.6-luna" || d.Capability != "cheap" {
		t.Fatalf("select_model consumer missed preferences %+v", d)
	}
	before, _ := os.ReadFile(logPath)
	previewed, e := routingPreview(svc, usage, nil)(host, json.RawMessage(`{"role":"scout"}`))
	if e != nil {
		t.Fatal(e)
	}
	p := previewed.(routingPreviewView)
	if p.Model != d.Model || p.Effort != d.Effort || p.Capability != d.Capability {
		t.Fatalf("preview mismatch %+v %+v", p, d)
	}
	after, _ := os.ReadFile(logPath)
	if string(before) != string(after) || events != 1 {
		t.Fatal("preview emitted audit")
	}
	encoded, _ := json.Marshal(p)
	for _, secret := range []string{"decisionId", "ceilings", "maxToolScope", path} {
		if strings.Contains(string(encoded), secret) {
			t.Errorf("preview exposed %s", secret)
		}
	}
	restarted := routing.New(path, nil)
	again, e := routingSelect(restarted, usage, nil, nil, nil)(host, json.RawMessage(`{"role":"scout"}`))
	if e != nil || again.(routing.Decision).Model != d.Model {
		t.Fatal("restart select lost preference")
	}
}
