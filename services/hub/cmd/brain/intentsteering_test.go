package main

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
)

func TestIntentDirectionTransportReceipts(t *testing.T) {
	for _, tc := range []struct {
		code       int
		body, want string
	}{{200, "{}", "accepted"}, {409, "{}", "failed"}, {500, "error", "unknown"}, {503, "session input queue is full", "failed"}} {
		t.Run(fmt.Sprint(tc.code), func(t *testing.T) {
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
				if req.URL.Path != "/sessions/worker/message" || req.Method != "POST" {
					t.Errorf("wrong delivery: %s %s", req.Method, req.URL.Path)
				}
				w.WriteHeader(tc.code)
				fmt.Fprint(w, tc.body)
			}))
			defer srv.Close()
			r := &registry{cm: newClaudemonClient(srv.URL)}
			raw, err := r.replacementHostCall(context.Background(), "intent.send", json.RawMessage(`{"sessionId":"worker","text":"Pinned direction"}`))
			if err != nil || !strings.Contains(string(raw), `"status":"`+tc.want+`"`) {
				t.Fatalf("receipt: %s %v", raw, err)
			}
		})
	}
}

func TestIntentDirectionPeerIsQualifiedWithoutLocalFallback(t *testing.T) {
	var called string
	r := &registry{callHub: func(_ context.Context, method string, params any) (json.RawMessage, error) {
		called = method
		encoded, _ := json.Marshal(params)
		if !strings.Contains(string(encoded), `"sessionId":"worker"`) {
			t.Fatalf("wrong target: %s", encoded)
		}
		return nil, fmt.Errorf("peer acknowledgment lost")
	}}
	_, err := r.replacementHostCall(context.Background(), "intent.send", json.RawMessage(`{"sessionId":"worker","hub":"peer-a","text":"Pinned direction"}`))
	if err == nil || called != "hub:peer-a/agents.sendMessage" {
		t.Fatalf("route=%q err=%v", called, err)
	}
	// cm is nil: any same-ID local fallback would panic.
}

func TestIntentInterruptUsesExistingSignalTransport(t *testing.T) {
	for _, code := range []int{200, 409} {
		t.Run(fmt.Sprint(code), func(t *testing.T) {
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
				if req.URL.Path != "/sessions/worker/signal" || req.Method != "POST" {
					t.Errorf("wrong signal target: %s %s", req.Method, req.URL.Path)
				}
				var p struct {
					Signal string `json:"signal"`
				}
				if json.NewDecoder(req.Body).Decode(&p) != nil || p.Signal != "SIGINT" {
					t.Errorf("wrong signal: %+v", p)
				}
				w.WriteHeader(code)
				fmt.Fprint(w, `{}`)
			}))
			defer srv.Close()
			r := &registry{cm: newClaudemonClient(srv.URL)}
			raw, err := r.replacementHostCall(context.Background(), "intent.interrupt", json.RawMessage(`{"sessionId":"worker"}`))
			if code == 200 && (err != nil || !strings.Contains(string(raw), `"status":"accepted"`)) {
				t.Fatalf("receipt: %s %v", raw, err)
			}
			if code != 200 && err == nil {
				t.Fatal("rejected signal was accepted")
			}
		})
	}
}

func TestIntentSteeringDesktopHostEndToEnd(t *testing.T) {
	bundle := os.Getenv("WKS_DESKTOP_HOST_TEST_BUNDLE")
	if bundle == "" {
		t.Skip("requires built desktop host bundle")
	}
	root := t.TempDir()
	home := filepath.Join(root, "home")
	if err := os.MkdirAll(home, 0755); err != nil {
		t.Fatal(err)
	}
	setHome(t, home)
	setConfigHome(t, filepath.Join(root, "config"))
	var code atomic.Int32
	code.Store(200)
	var sends atomic.Int32
	packets := make(chan string, 8)
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		if req.URL.Path == "/sessions/worker/message" {
			var body struct {
				Text string `json:"text"`
			}
			_ = json.NewDecoder(req.Body).Decode(&body)
			packets <- body.Text
			sends.Add(1)
			w.WriteHeader(int(code.Load()))
			fmt.Fprint(w, `{}`)
			return
		}
		fmt.Fprint(w, `[]`)
	}))
	defer srv.Close()
	r := newRegistry(newClaudemonClient(srv.URL))
	r.desktopServices.bundle = bundle
	defer r.desktopServices.close()
	r.store = newSessionStore()
	r.store.enrich = func(raw json.RawMessage) json.RawMessage { return enrichAndCompat(raw, r.meta) }
	worker, _ := json.Marshal(map[string]any{"session_id": "worker", "cwd": home, "mode": "input", "provider": "codex"})
	r.store.set("worker", worker)
	call := func(input map[string]any) map[string]any {
		t.Helper()
		params, _ := json.Marshal(map[string]any{"request": input})
		raw, err := r.desktopInternalCall(context.Background(), "desktop.intentWorkspaceRequest", params)
		if err != nil {
			t.Fatalf("request %v failed: %v", input, err)
		}
		var out map[string]any
		if err = json.Unmarshal(raw, &out); err != nil {
			t.Fatal(err)
		}
		return out
	}
	fields := map[string]any{"title": "Feature", "outcome": "Export", "constraints": "CSV only", "successCriteria": "Checks pass", "sourceUrl": "", "status": "active"}
	created := call(map[string]any{"action": "create", "projectRoot": home, "fields": fields})
	id := created["workspace"].(map[string]any)["id"]
	attached := call(map[string]any{"action": "attachSession", "id": id, "expectedRevision": 1, "session": map[string]string{"sessionId": "worker", "hub": "", "label": "Worker", "provider": "codex", "cwd": home}})
	executionID := attached["execution"].(map[string]any)["id"]
	prepare := func(directionID string) map[string]any {
		return call(map[string]any{"action": "prepareDirection", "id": id, "directionId": directionID, "executionId": executionID, "expectedRevision": 1, "text": "Keep permission checks"})
	}
	send := func(directionID, attemptID string) map[string]any {
		return call(map[string]any{"action": "sendDirection", "id": id, "directionId": directionID, "attemptId": attemptID})
	}
	status := func(result map[string]any) string {
		attempts := result["direction"].(map[string]any)["attempts"].([]any)
		return attempts[len(attempts)-1].(map[string]any)["status"].(string)
	}
	prepared := prepare("first")
	if sends.Load() != 0 {
		t.Fatal("saving sent a message")
	}
	if status(send("first", "attempt-one")) != "accepted" {
		t.Fatal("missing acceptance")
	}
	if got := <-packets; got != prepared["direction"].(map[string]any)["packet"] {
		t.Fatalf("sent different packet: %s", got)
	}
	send("first", "attempt-one")
	send("first", "other-attempt")
	if sends.Load() != 1 {
		t.Fatal("accepted delivery replayed")
	}
	prepare("second")
	code.Store(409)
	if status(send("second", "failure")) != "failed" {
		t.Fatal("refusal was not recorded")
	}
	code.Store(200)
	if status(send("second", "explicit-retry")) != "accepted" {
		t.Fatal("explicit retry failed")
	}
	prepare("third")
	code.Store(500)
	if status(send("third", "uncertain")) != "unknown" {
		t.Fatal("uncertainty was not recorded")
	}
	r.desktopServices.close()
	previousStore := r.store
	r = newRegistry(newClaudemonClient(srv.URL))
	r.desktopServices.bundle = bundle
	r.store = previousStore
	defer r.desktopServices.close()
	before := sends.Load()
	send("third", "after-restart")
	if sends.Load() != before {
		t.Fatal("uncertain delivery replayed after restart")
	}
	if len(call(map[string]any{"action": "directions", "id": id})["directions"].([]any)) != 3 {
		t.Fatal("lost direction history")
	}
}

func TestIntentActivationUsesManagerSpawnWithoutCallerGrants(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)
	reg.meta = newMetaStore()
	reg.mcpFacadeURL = "http://127.0.0.1:7897/mcp"
	result, err := reg.replacementHostCall(context.Background(), "intent.spawn", []byte(`{"cwd":"/tmp","provider":"codex","label":"Intent: Export","message":"Pursue the saved intent","yoloGranted":true,"skipPermissions":true}`))
	if err != nil {
		t.Fatal(err)
	}
	calls := rec.calls("/sessions/spawn-managed")
	if len(calls) != 1 {
		t.Fatalf("expected one manager, got %d", len(calls))
	}
	if calls[0].body["first_message"] != "Pursue the saved intent" {
		t.Fatalf("lost kickoff: %+v", calls[0].body)
	}
	if strings.Contains(string(result), `"fullAccess":true`) {
		t.Fatalf("caller grants leaked: %s", result)
	}
	var response struct {
		SessionID string `json:"sessionId"`
	}
	if err := json.Unmarshal(result, &response); err != nil || response.SessionID == "" {
		t.Fatalf("no manager identity: %s %v", result, err)
	}
	if meta, ok := reg.meta.get(response.SessionID); !ok || !meta.IsWakeTarget {
		t.Fatalf("manager cannot receive worker wakes: %+v", meta)
	}
}
