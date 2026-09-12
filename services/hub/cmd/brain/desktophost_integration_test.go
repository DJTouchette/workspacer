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
)

// Explicit cross-stack suite: npm's test:desktop-host supplies the built bundle.
// Plain Go-only builds do not pretend to test a TypeScript artifact they lack.
func TestDesktopHostEndToEnd(t *testing.T) {
	bundle := os.Getenv("WKS_DESKTOP_HOST_TEST_BUNDLE")
	if bundle == "" {
		t.Skip("run npm run test:desktop-host for the real Node/brain integration")
	}
	if !filepath.IsAbs(bundle) {
		t.Fatal("test bundle must be absolute")
	}
	root := t.TempDir()
	home := filepath.Join(root, "home")
	repo := filepath.Join(home, "project")
	if err := os.MkdirAll(repo, 0755); err != nil {
		t.Fatal(err)
	}
	setHome(t, home)
	setConfigHome(t, filepath.Join(root, "config"))
	t.Setenv("CLAUDE_CONFIG_DIR", filepath.Join(home, ".claude"))
	fixtureGit(t, repo, "init", "-q")
	fixtureGit(t, repo, "config", "user.name", "Test")
	fixtureGit(t, repo, "config", "user.email", "test@example.invalid")
	fixtureGit(t, repo, "config", "commit.gpgsign", "false")
	if err := os.WriteFile(filepath.Join(repo, "a.txt"), []byte("original\n"), 0600); err != nil {
		t.Fatal(err)
	}
	fixtureGit(t, repo, "add", "a.txt")
	fixtureGit(t, repo, "commit", "-qm", "initial")
	library := filepath.Join(configDir(), "library")
	if err := os.MkdirAll(library, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(library, "ship.md"), []byte("---\nname: Ship\nkind: dispatch\n---\n{{task}}\nExecute in {{cwd}}; project {{projectCwd}}.\n"), 0600); err != nil {
		t.Fatal(err)
	}
	launches := make(chan spawnManagedReq, 4)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if req.URL.Path == "/sessions/spawn-managed" {
			var p spawnManagedReq
			if err := json.NewDecoder(req.Body).Decode(&p); err != nil {
				t.Error(err)
				w.WriteHeader(400)
				return
			}
			launches <- p
			_ = json.NewEncoder(w).Encode(map[string]any{"session_id": p.SessionID, "first_message_queued": true})
			return
		}
		if req.URL.Path == "/sessions" {
			_, _ = w.Write([]byte(`[]`))
			return
		}
		_, _ = w.Write([]byte(`{}`))
	}))
	defer srv.Close()
	r := newRegistry(newClaudemonClient(srv.URL))
	r.desktopServices.bundle = bundle
	defer r.desktopServices.close()
	r.meta = newMetaStore()
	r.store = newSessionStore()
	r.store.enrich = func(raw json.RawMessage) json.RawMessage { return enrichAndCompat(raw, r.meta) }
	r.meta.set("manager", spawnMeta{IsWakeTarget: true, Label: "Manager"})
	manager, _ := json.Marshal(map[string]any{"session_id": "manager", "cwd": repo, "mode": "input"})
	r.store.set("manager", manager)
	items := listLibrary("", libraryFileGuardFor("library.list", ""), libraryFilter{Kind: "dispatch"})
	template := ""
	for _, item := range items {
		if strings.Contains(item.Body, "Execute in") {
			template = item.ID
		}
	}
	if template == "" {
		t.Fatal("fixture template was not discovered")
	}
	request, _ := json.Marshal(map[string]any{"cwd": repo, "provider": "claude", "transport": "stream", "label": "Worker", "parentSessionId": "manager", "dispatchOwnerSessionId": "manager", "template": template, "templateParams": map[string]string{"task": "Implement the real delta"}, "worktree": true, "resultSchema": map[string]any{"type": "object", "required": []string{"status"}, "properties": map[string]any{"status": map[string]string{"type": "string"}}}})
	raw, err := r.handle(context.Background(), "agents.spawn", request)
	if err != nil {
		t.Fatal(err)
	}
	var receipt struct {
		SessionID string `json:"sessionId"`
		TaskID    string `json:"taskId"`
		Queued    bool   `json:"messageQueued"`
	}
	if err = json.Unmarshal(raw, &receipt); err != nil {
		t.Fatal(err)
	}
	if receipt.SessionID == "" || receipt.TaskID == "" || !receipt.Queued {
		t.Fatalf("missing launch receipt: %s", raw)
	}
	launch := <-launches
	if launch.Cwd == repo || !strings.Contains(launch.FirstMessage, launch.Cwd) || !strings.Contains(launch.FirstMessage, "Implement the real delta") || !strings.Contains(launch.Instructions, "wks-result") {
		t.Fatalf("template/schema/execution drift: %+v", launch)
	}
	if err = os.WriteFile(filepath.Join(launch.Cwd, "a.txt"), []byte("worker change\n"), 0600); err != nil {
		t.Fatal(err)
	}
	fixtureGit(t, launch.Cwd, "add", "a.txt")
	fixtureGit(t, launch.Cwd, "commit", "-qm", "worker change")
	finished, _ := json.Marshal(map[string]any{"session_id": receipt.SessionID, "cwd": launch.Cwd, "mode": "stopped"})
	r.store.set(receipt.SessionID, finished)
	if _, err = r.desktopInternalCall(context.Background(), "internal.observe", nil); err != nil {
		t.Fatal(err)
	}
	entry := fleetEntry{SessionID: receipt.SessionID, Label: "Worker"}
	r.attachDesktopResult(context.Background(), &entry, "Done.\n```wks-result\n{\"status\":\"done\"}\n```")
	if entry.ResultError != "" || entry.Result == "" || entry.ReviewEvidenceID == "" {
		t.Fatalf("missing validated completion: %+v", entry)
	}
	history, err := r.handle(context.Background(), "desktop.dispatchHistoryRead", nil)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(history), `"resultContract":"valid"`) || !strings.Contains(string(history), entry.ReviewEvidenceID) {
		t.Fatalf("history lost result evidence: %s", history)
	}
	// A forged owner must fail before another real daemon spawn happens.
	var bad map[string]any
	_ = json.Unmarshal(request, &bad)
	bad["taskId"] = receipt.TaskID
	bad["dispatchOwnerSessionId"] = "forged"
	broken, _ := json.Marshal(bad)
	if _, err = r.handle(context.Background(), "agents.spawn", broken); err == nil {
		t.Fatal("forged task ownership was admitted")
	}
	if len(launches) != 0 {
		t.Fatal("rejected task created a daemon process")
	}
	r.callHub = func(_ context.Context, method string, params any) (json.RawMessage, error) {
		if method != "plugins.prepareLaunch" {
			t.Fatalf("unexpected integration callback %s", method)
		}
		encoded, _ := json.Marshal(params)
		var callback struct {
			CallID   string         `json:"callId"`
			PluginID string         `json:"pluginId"`
			Op       string         `json:"op"`
			Context  map[string]any `json:"context"`
		}
		_ = json.Unmarshal(encoded, &callback)
		if callback.CallID != "fixture-call" || callback.PluginID != "test.integration" {
			t.Fatal("callback lost its private spawn identity")
		}
		if callback.Op == "describe" {
			return json.RawMessage(`[{"id":"test.integration","launchIntegration":{"version":1,"agents":["claude"],"prepareMethod":"test.integration.prepare"},"provides":["test.integration.prepare"]}]`), nil
		}
		if len(callback.Context) > 5 || callback.Context["cwd"] != repo || callback.Context["agent"] != "claude" {
			t.Fatalf("integration leaked or changed launch context: %s", encoded)
		}
		return json.RawMessage(`{"env":{"INTEGRATION_TEST":"yes"},"args":["--fixture-route"]}`), nil
	}
	integrated, _ := json.Marshal(map[string]any{"cwd": repo, "provider": "claude", "transport": "stream", "launchIntegrationId": "test.integration", "launchIntegrationGranted": true})
	if _, err := r.handle(context.WithValue(context.Background(), inboundCallIDKey{}, "fixture-call"), "agents.spawn", integrated); err != nil {
		t.Fatal(err)
	}
	patched := <-launches
	if patched.Env["INTEGRATION_TEST"] != "yes" || !strings.Contains(strings.Join(patched.ExtraArgs, " "), "--fixture-route") {
		t.Fatal("plugin launch patch did not reach the daemon")
	}

}
