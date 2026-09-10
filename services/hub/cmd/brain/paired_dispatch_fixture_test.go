package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/event"
	"github.com/djtouchette/workspacer-hub/internal/limits"
	"github.com/djtouchette/workspacer-hub/internal/routing"
)

// Hosted cross-language fixture: real paired bus authorization and production
// brain handlers/watchers. Only the provider process and its output are fake.
func TestPairedDispatchHostFixture(t *testing.T) {
	if os.Getenv("WKS_PAIRED_CHAIN_FIXTURE") != "1" {
		return
	}
	root := t.TempDir()
	t.Setenv("HOME", root)
	t.Setenv("XDG_CONFIG_HOME", root)
	t.Setenv("APPDATA", root)
	if err := os.MkdirAll(configDir(), 0700); err != nil {
		t.Fatal(err)
	}
	repo := filepath.Join(root, "remote-repo")
	nonRepo := filepath.Join(root, "remote-non-repo")
	_ = os.MkdirAll(nonRepo, 0700)
	for _, f := range []string{"frontend/tracked.ts", "backend/tracked.go"} {
		p := filepath.Join(repo, f)
		_ = os.MkdirAll(filepath.Dir(p), 0700)
		_ = os.WriteFile(p, []byte("fixture\n"), 0600)
	}
	gitInit(t, repo)
	cli := filepath.Join(root, "claude-fixture")
	_ = os.WriteFile(cli, []byte("#!/bin/sh\n[ \"$*\" = \"auth status\" ] || exit 2\nprintf '%s\\n' '{\"loggedIn\":true}'\n"), 0700)
	config, _ := json.Marshal(map[string]any{"agents": map[string]any{"binaries": map[string]string{"claude": cli, "codex": filepath.Join(root, "missing-codex")}}, "projects": map[string]any{repo: map[string]any{}, nonRepo: map[string]any{}}})
	_ = os.WriteFile(configPath(), config, 0600)
	var mu sync.Mutex
	var launches []map[string]any
	var sid, cwd, reply string
	var reg *registry
	var replayMode atomic.Int32
	daemon := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		if req.URL.Path == "/control" {
			var command struct {
				Kind  string `json:"kind"`
				Reply string `json:"reply"`
			}
			_ = json.NewDecoder(req.Body).Decode(&command)
			if strings.HasPrefix(command.Kind, "replay-") {
				switch command.Kind {
				case "replay-unknown":
					replayMode.Store(1)
				case "replay-error":
					replayMode.Store(2)
				case "replay-known":
					replayMode.Store(0)
				case "replay-wrong-id":
					replayMode.Store(3)
				default:
					http.Error(w, "unknown fixture replay mode", http.StatusBadRequest)
					return
				}
				_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
				return
			}

			mu.Lock()
			id, dir := sid, cwd
			reply = command.Reply
			mu.Unlock()
			if command.Kind == "handoff-result" {
				if err := os.WriteFile(filepath.Join(dir, "implementation.txt"), []byte("result C\n"), 0600); err != nil {
					t.Error(err)
				}
				handoffFixtureGit(t, dir, "add", "implementation.txt")
				handoffFixtureGit(t, dir, "commit", "-m", "workspace fixture result C")
				reg.remote.mu.Lock()
				dispatch := reg.remote.bySession[id]
				reg.remote.mu.Unlock()
				if err := os.WriteFile(filepath.Join(dir, ".workspacer", "handoffs", dispatch, "implementation.md"), []byte("# Result evidence\nSynthetic provider claim: tests passed.\n"), 0600); err != nil {
					t.Error(err)
				}
			}
			mode := "idle"
			if command.Kind == "block" {
				mode = "waiting_approval"
			}
			if command.Kind == "progress" {
				p, _ := json.Marshal(map[string]string{"callerSessionId": id, "note": "remote fixture progress"})
				_, _ = reg.reportProgress(context.Background(), p)
			} else {
				stream, _ := json.Marshal(map[string]any{"sessionId": id, "cwd": dir, "status": "active", "ambientState": "streaming", "provider": "claude"})
				reg.store.set(id, stream)
				finished, _ := json.Marshal(map[string]any{"sessionId": id, "cwd": dir, "status": "active", "ambientState": mode, "provider": "claude", "pendingApproval": command.Kind == "block"})
				reg.store.set(id, finished)
			}
			_ = json.NewEncoder(w).Encode(map[string]bool{"ok": true})
			return
		}
		if req.URL.Path == "/evidence" {
			mu.Lock()
			defer mu.Unlock()
			_ = json.NewEncoder(w).Encode(launches)
			return
		}
		if strings.HasSuffix(req.URL.Path, "/conversation") {
			mu.Lock()
			text := reply
			mu.Unlock()
			_, _ = io.WriteString(w, dispatched(text))
			return
		}
		if req.URL.Path == "/sessions/spawn-managed" || req.URL.Path == "/sessions/spawn" {
			var p map[string]any
			_ = json.NewDecoder(req.Body).Decode(&p)
			mu.Lock()
			launches = append(launches, p)
			sid, _ = p["session_id"].(string)
			cwd, _ = p["cwd"].(string)
			id := sid
			mu.Unlock()
			snap, _ := json.Marshal(map[string]any{"sessionId": id, "cwd": p["cwd"], "status": "active", "ambientState": "streaming", "provider": "claude"})
			reg.store.set(id, snap)
			_ = json.NewEncoder(w).Encode(map[string]any{"session_id": id, "first_message_queued": true})
			return
		}
		_, _ = io.WriteString(w, `{"ok":true}`)
	}))
	defer daemon.Close()
	b := broker.New()
	srv := bus.NewServer(b)
	srv.SetToken("paired-fixture-host")
	srv.SetScopedTokenLookup(func(token string) (bus.ScopedIdent, bool) {
		switch token {
		case "paired-fixture-operator":
			return bus.ScopedIdent{Scope: "operator", Methods: authtoken.ScopeOperator.Methods()}, true
		case "paired-fixture-view":
			return bus.ScopedIdent{Scope: "view", Methods: authtoken.ScopeView.Methods()}, true
		}
		return bus.ScopedIdent{}, false
	})
	reg = newRegistry(newClaudemonClient(daemon.URL))
	reg.scope = "full"
	reg.mcpFacadeURL = daemon.URL + "/mcp"
	reg.meta = newMetaStore()
	reg.store = newSessionStore()
	reg.store.enrich = func(raw json.RawMessage) json.RawMessage { return enrichAndCompat(raw, reg.meta) }
	reg.remote = newRemoteDispatchStore()
	if err := reg.remote.load(filepath.Join(configDir(), "remote-dispatches.json")); err != nil {
		t.Fatal(err)
	}
	reg.publish = func(topic string, data json.RawMessage) { b.Publish(event.New(topic, "brain-fixture", data)) }
	reg.fin = newFinishWatcher(reg)
	reg.fin.coalesce = time.Millisecond
	reg.fin.blocks.debounce = time.Millisecond
	reg.store.onChange = func(_ string, raw json.RawMessage) { reg.fin.observe(context.Background(), raw) }
	matrix, err := routing.Defaults()
	if err != nil {
		t.Fatal(err)
	}
	matrix.ActiveProfile = "anthropic_only"
	srv.RegisterLocal("routing.select", func(raw json.RawMessage) (any, error) {
		var req routing.Request
		if err := json.Unmarshal(raw, &req); err != nil {
			return nil, err
		}
		decision := routing.Select(matrix, limits.Snapshot{}, nil, nil, time.Now(), req)
		decision.DecisionID = routing.NewDecisionID()
		return decision, nil
	})
	for _, method := range reg.methods() {
		method := method
		srv.RegisterLocal(method, func(raw json.RawMessage) (any, error) {
			if method == "agents.dispatchReplay" {
				switch replayMode.Load() {
				case 1:
					// Exercise the production response for an unavailable journal
					// on the real authenticated paired socket, without racing the
					// active worker's registry or emitting a fabricated finish.
					unavailable := &registry{}
					return unavailable.dispatchReplay(context.Background(), raw)
				case 2:
					return nil, fmt.Errorf("fixture replay transport failure")
				case 3:
					return jsonResult(map[string]any{"state": "unknown", "dispatchId": "unrelated-dispatch-nonce"})
				}
			}
			return reg.handle(context.Background(), method, raw)
		})
	}
	oldServer := bus.NewServer(broker.New())
	oldServer.SetToken("paired-fixture-operator")
	oldHost := httptest.NewServer(oldServer.Handler())
	defer oldHost.Close()
	host := httptest.NewServer(srv.Handler())
	defer host.Close()
	ready := map[string]string{"url": host.URL, "oldURL": oldHost.URL, "repo": repo, "nonRepo": nonRepo, "control": daemon.URL}
	for key, value := range configureHandoffChainFixture(t, reg, repo) {
		ready[key] = value
	}
	_ = json.NewEncoder(os.Stdout).Encode(ready)
	_, _ = io.Copy(io.Discard, os.Stdin)
}
