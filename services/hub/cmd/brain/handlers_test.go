package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// These tests stand up a fake claudemon with httptest and verify the handlers
// forward the right requests — real verification of the routing/payloads without
// a live daemon or hub.

func TestSpawnForwardsArgvAndReturnsSessionID(t *testing.T) {
	var gotPath string
	var gotBody spawnReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": gotBody.SessionID})
	}))
	defer srv.Close()

	reg := newRegistry(newClaudemonClient(srv.URL))
	params := []byte(`{"cwd":"/tmp/proj/","skipPermissions":true}`)
	res, err := reg.handle(context.Background(), "agents.spawn", params)
	if err != nil {
		t.Fatal(err)
	}

	if gotPath != "/sessions/spawn" {
		t.Fatalf("posted to %q, want /sessions/spawn", gotPath)
	}
	if gotBody.Cwd != "/tmp/proj" {
		t.Errorf("cwd = %q, want normalized /tmp/proj", gotBody.Cwd)
	}
	if gotBody.Argv[0] != "claude" {
		t.Errorf("argv[0] = %q, want claude", gotBody.Argv[0])
	}
	if !containsStr(gotBody.Argv, "--dangerously-skip-permissions") {
		t.Errorf("argv missing skip flag: %v", gotBody.Argv)
	}
	if !containsPair(gotBody.Argv, "--session-id", gotBody.SessionID) || gotBody.SessionID == "" {
		t.Errorf("argv should pin --session-id <id>: %v", gotBody.Argv)
	}

	var out struct {
		SessionID string `json:"sessionId"`
	}
	if err := json.Unmarshal(res, &out); err != nil || out.SessionID != gotBody.SessionID {
		t.Errorf("result = %s, want sessionId %q (err %v)", res, gotBody.SessionID, err)
	}
}

func TestSendMessageForwards(t *testing.T) {
	var gotPath, gotText string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		var body struct {
			Text string `json:"text"`
		}
		_ = json.NewDecoder(r.Body).Decode(&body)
		gotText = body.Text
		w.Write([]byte(`{"ok":true}`))
	}))
	defer srv.Close()

	reg := newRegistry(newClaudemonClient(srv.URL))
	_, err := reg.handle(context.Background(), "agents.sendMessage",
		[]byte(`{"sessionId":"s1","text":"hello"}`))
	if err != nil {
		t.Fatal(err)
	}
	if gotPath != "/sessions/s1/message" || gotText != "hello" {
		t.Fatalf("forwarded to %q text %q", gotPath, gotText)
	}
}

func TestSendMessageRequiresFields(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))
	if _, err := reg.handle(context.Background(), "agents.sendMessage", []byte(`{"sessionId":"s1"}`)); err == nil {
		t.Fatal("expected error for missing text")
	}
}

func TestListRelaysRawBody(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/sessions" {
			t.Errorf("unexpected path %q", r.URL.Path)
		}
		w.Write([]byte(`[{"sessionId":"a"},{"sessionId":"b"}]`))
	}))
	defer srv.Close()

	reg := newRegistry(newClaudemonClient(srv.URL))
	res, err := reg.handle(context.Background(), "agents.list", nil)
	if err != nil {
		t.Fatal(err)
	}
	if strings.TrimSpace(string(res)) != `[{"sessionId":"a"},{"sessionId":"b"}]` {
		t.Fatalf("list should relay claudemon body verbatim, got %s", res)
	}
}

func TestClaudemonErrorSurfaces(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusBadRequest)
		io.WriteString(w, "no such session")
	}))
	defer srv.Close()

	reg := newRegistry(newClaudemonClient(srv.URL))
	_, err := reg.handle(context.Background(), "claude.signal", []byte(`{"sessionId":"x","signal":"SIGINT"}`))
	if err == nil || !strings.Contains(err.Error(), "no such session") {
		t.Fatalf("expected claudemon error to surface, got %v", err)
	}
}

func TestUnknownMethodErrors(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))
	if _, err := reg.handle(context.Background(), "does.not.exist", nil); err == nil {
		t.Fatal("expected error for unknown method")
	}
}

func TestAnswerRequiresOptionOrText(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://unused"))
	if _, err := reg.handle(context.Background(), "claude.answer", []byte(`{"sessionId":"s1"}`)); err == nil {
		t.Fatal("expected error when neither option nor text given")
	}
}

func containsStr(s []string, v string) bool {
	for _, x := range s {
		if x == v {
			return true
		}
	}
	return false
}

func containsPair(s []string, a, b string) bool {
	for i := 0; i+1 < len(s); i++ {
		if s[i] == a && s[i+1] == b {
			return true
		}
	}
	return false
}
