package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestHeadlessManagerReplacementEndToEnd(t *testing.T) {
	bundle := os.Getenv("WKS_DESKTOP_HOST_TEST_BUNDLE")
	if bundle == "" {
		t.Skip("run npm run test:desktop-host")
	}
	root := t.TempDir()
	home := filepath.Join(root, "home")
	if err := os.MkdirAll(home, 0755); err != nil {
		t.Fatal(err)
	}
	setHome(t, home)
	setConfigHome(t, filepath.Join(root, "config"))
	t.Setenv("CLAUDE_CONFIG_DIR", filepath.Join(home, ".claude"))
	var mu sync.Mutex
	rows := map[string]map[string]any{}
	replies := map[string]string{}
	messages := map[string][]string{}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		switch req.URL.Path {
		case "/health":
			fmt.Fprint(w, `{}`)
			return
		case "/sessions":
			out := []map[string]any{}
			for _, row := range rows {
				out = append(out, row)
			}
			_ = json.NewEncoder(w).Encode(out)
			return
		case "/sessions/spawn-managed":
			var p spawnManagedReq
			if err := json.NewDecoder(req.Body).Decode(&p); err != nil {
				t.Error(err)
				w.WriteHeader(400)
				return
			}
			rows[p.SessionID] = map[string]any{"session_id": p.SessionID, "provider": p.Provider, "cwd": p.Cwd, "transport": "stream", "mode": "input", "user_prompts": 0, "requested_selection": map[string]any{"model": p.ModelIdentity, "context_window": p.ContextWindow}, "started_at": time.Now().UTC().Format(time.RFC3339), "updated_at": time.Now().UTC().Format(time.RFC3339)}
			_ = json.NewEncoder(w).Encode(map[string]any{"session_id": p.SessionID, "first_message_queued": false})
			return
		}
		parts := strings.Split(strings.Trim(req.URL.Path, "/"), "/")
		if len(parts) < 2 || parts[0] != "sessions" {
			fmt.Fprint(w, `{}`)
			return
		}
		id := parts[1]
		row := rows[id]
		if row == nil {
			w.WriteHeader(404)
			return
		}
		if len(parts) == 2 {
			_ = json.NewEncoder(w).Encode(row)
			return
		}
		switch parts[2] {
		case "conversation":
			_ = json.NewEncoder(w).Encode(map[string]any{"items": []map[string]string{{"kind": "user_message", "text": "Manage the project"}, {"kind": "assistant_text", "text": replies[id]}}})
			return
		case "signal":
			row["mode"] = "stopped"
			fmt.Fprint(w, `{}`)
			return
		case "message":
			var body struct {
				Text string `json:"text"`
			}
			_ = json.NewDecoder(req.Body).Decode(&body)
			messages[id] = append(messages[id], body.Text)
			row["user_prompts"] = row["user_prompts"].(int) + 1
			if strings.Contains(body.Text, "This is preparation only") {
				start := strings.Index(body.Text, "\n{\n")
				var artifact map[string]any
				if start < 0 || json.NewDecoder(strings.NewReader(body.Text[start+1:])).Decode(&artifact) != nil {
					t.Error("missing artifact example")
					w.WriteHeader(400)
					return
				}
				brief := filepath.Join(home, ".workspacer", "brief.md")
				_ = os.MkdirAll(filepath.Dir(brief), 0700)
				_ = os.WriteFile(brief, []byte("# Now\nPreserve the pending work.\n"), 0600)
				contents, _ := os.ReadFile(brief)
				hash := sha256.Sum256(contents)
				artifact["checkpoint"] = map[string]any{"completed": true, "files": []map[string]string{{"path": brief, "sha256": hex.EncodeToString(hash[:])}}}
				artifact["nextAction"] = "Inspect the active worker result"
				data, _ := json.Marshal(artifact)
				file := filepath.Join(home, ".workspacer", "manager-handoffs", artifact["operationId"].(string), "handoff.json")
				if err := os.WriteFile(file, data, 0600); err != nil {
					t.Error(err)
				}
				digest := sha256.Sum256(data)
				receipt, _ := json.Marshal(map[string]any{"operationId": artifact["operationId"], "sourceSessionId": id, "sha256": hex.EncodeToString(digest[:])})
				replies[id] = "```wks-manager-handoff\n" + string(receipt) + "\n```"
			}
			fmt.Fprint(w, `{"ok":true}`)
			return
		}
		w.WriteHeader(404)
	}))
	defer srv.Close()
	r := newRegistry(newClaudemonClient(srv.URL))
	r.meta = newMetaStore()
	r.store = newSessionStore()
	r.store.enrich = func(raw json.RawMessage) json.RawMessage { return enrichAndCompat(raw, r.meta) }
	r.desktopServices.bundle = bundle
	r.mcpFacadeURL = srv.URL + "/mcp"
	defer r.desktopServices.close()
	call := func(method string, params any) json.RawMessage {
		t.Helper()
		raw, _ := json.Marshal(params)
		result, err := r.handle(context.Background(), method, raw)
		if err != nil {
			t.Fatalf("%s: %v", method, err)
		}
		return result
	}
	spawned := call("agents.spawn", map[string]any{"cwd": home, "provider": "claude", "transport": "stream", "model": "claude-sonnet-4-5", "manager": true, "toolScope": "operator"})
	var first struct {
		SessionID    string `json:"sessionId"`
		HistoryError string `json:"historyError"`
	}
	_ = json.Unmarshal(spawned, &first)
	if first.SessionID == "" || first.HistoryError != "" {
		t.Fatalf("manager launch failed: %s", spawned)
	}
	mu.Lock()
	sourceRow, _ := json.Marshal(rows[first.SessionID])
	mu.Unlock()
	r.store.set(first.SessionID, sourceRow)
	workerResult := call("agents.spawn", map[string]any{"cwd": home, "provider": "claude", "transport": "stream", "label": "Worker", "parentSessionId": first.SessionID, "dispatchOwnerSessionId": first.SessionID})
	var worker struct {
		SessionID string `json:"sessionId"`
		TaskID    string `json:"taskId"`
	}
	_ = json.Unmarshal(workerResult, &worker)
	if worker.TaskID == "" {
		t.Fatalf("missing worker task: %s", workerResult)
	}
	mu.Lock()
	rows[worker.SessionID]["mode"] = "working"
	workerRow, _ := json.Marshal(rows[worker.SessionID])
	mu.Unlock()
	r.store.set(worker.SessionID, workerRow)
	request := func(action string, operation string, session string) json.RawMessage {
		input := map[string]any{"action": action}
		if action == "start" {
			input["sourceSessionId"] = first.SessionID
			input["paneId"] = "pane"
			input["workspaceId"] = "workspace"
		} else if operation != "" {
			input["operationId"] = operation
		}
		return call("desktop.managerReplacement", map[string]any{"request": input, "bindings": [][]string{{"pane", session}}})
	}
	response := request("start", "", first.SessionID)
	var state struct {
		Available  bool   `json:"available"`
		Error      string `json:"error"`
		Operations []struct {
			OperationID string `json:"operationId"`
			Successor   string `json:"successorSessionId"`
			Phase       string `json:"phase"`
			Error       string `json:"error"`
		} `json:"operations"`
	}
	decode := func(data []byte) {
		t.Helper()
		if err := json.Unmarshal(data, &state); err != nil {
			t.Fatal(err)
		}
		if !state.Available || state.Error != "" {
			t.Fatalf("handoff response: %s", data)
		}
	}
	decode(response)
	deadline := time.Now().Add(10 * time.Second)
	for len(state.Operations) == 0 || state.Operations[0].Phase != "binding" {
		if time.Now().After(deadline) {
			t.Fatalf("handoff did not reach binding: %s", response)
		}
		time.Sleep(25 * time.Millisecond)
		response = request("list", "", first.SessionID)
		decode(response)
	}
	op := state.Operations[0]
	// A message arriving during the transfer stays held until viewer bind.
	if ok, err := r.submitMessage(context.Background(), first.SessionID, "Retain this exact queued message"); !ok || err != nil {
		t.Fatalf("held message: %v %v", ok, err)
	}
	response = request("bind", op.OperationID, op.Successor)
	decode(response)
	for len(state.Operations) != 1 || state.Operations[0].Phase != "complete" {
		if time.Now().After(deadline) {
			t.Fatalf("handoff not complete: %s", response)
		}
		time.Sleep(25 * time.Millisecond)
		response = request("list", "", op.Successor)
		decode(response)
	}
	history := call("desktop.dispatchHistoryRead", map[string]any{})
	if !bytes.Contains(history, []byte(`"ownerSessionId":"`+op.Successor+`"`)) {
		t.Fatalf("task ownership was not transferred: %s", history)
	}
	meta, _ := r.meta.get(worker.SessionID)
	if meta.ParentSessionID != op.Successor {
		t.Fatal("worker ownership was not transferred")
	}
	mu.Lock()
	defer mu.Unlock()
	if rows[first.SessionID]["mode"] != "stopped" {
		t.Fatal("predecessor was not retired")
	}
	combined := strings.Join(messages[op.Successor], "\n")
	if !strings.Contains(combined, "HOST-OWNED MANAGER HANDOFF") || bytes.Count([]byte(combined), []byte("Retain this exact queued message")) != 1 {
		t.Fatalf("successor lost kickoff or held message: %s", combined)
	}
}
