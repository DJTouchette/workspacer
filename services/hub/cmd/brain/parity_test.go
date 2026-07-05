package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync"
	"testing"
)

// recorder is a fake claudemon that records the requests it receives and lets a
// test script per-path responses.
type recorder struct {
	mu     sync.Mutex
	hits   []hit
	status map[string]int // path → status code (default 200)
}

type hit struct {
	path string
	body map[string]any
}

func newRecorder() *recorder { return &recorder{status: map[string]int{}} }

func (rec *recorder) server() *httptest.Server {
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		rec.mu.Lock()
		rec.hits = append(rec.hits, hit{path: r.URL.Path, body: body})
		code := rec.status[r.URL.Path]
		rec.mu.Unlock()
		if code != 0 {
			w.WriteHeader(code)
		}
		// A spawn echoes back a session id; everything else is fine with {ok}.
		if r.URL.Path == "/sessions/spawn" {
			id, _ := body["session_id"].(string)
			if id == "" {
				id = "generated-id"
			}
			_ = json.NewEncoder(w).Encode(map[string]string{"session_id": id})
			return
		}
		w.Write([]byte(`{"ok":true}`))
	}))
}

func (rec *recorder) calls(path string) []hit {
	rec.mu.Lock()
	defer rec.mu.Unlock()
	var out []hit
	for _, h := range rec.hits {
		if h.path == path {
			out = append(out, h)
		}
	}
	return out
}

// ── claude.answer types into the PTY (matching the app), not /answer ────────

func TestAnswerOptionTypesIntoPTY(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.answer", []byte(`{"sessionId":"s1","option":2}`)); err != nil {
		t.Fatal(err)
	}
	in := rec.calls("/sessions/s1/input")
	if len(in) != 1 || in[0].body["text"] != "2\r" {
		t.Fatalf("expected one input of \"2\\r\", got %+v", in)
	}
	if len(rec.calls("/sessions/s1/answer")) != 0 {
		t.Fatal("answer must not hit the mode-gated /answer endpoint")
	}
}

func TestAnswerMultiPart(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.answer", []byte(`{"sessionId":"s1","answers":["yes","blue"]}`)); err != nil {
		t.Fatal(err)
	}
	in := rec.calls("/sessions/s1/input")
	if len(in) != 2 || in[0].body["text"] != "yes\r" || in[1].body["text"] != "blue\r" {
		t.Fatalf("expected two typed answers, got %+v", in)
	}
}

// ── sendMessage surfaces a 409 (ended session) instead of typing blind ──────

func TestSendMessageErrorsOn409(t *testing.T) {
	rec := newRecorder()
	rec.status["/sessions/s1/message"] = http.StatusConflict
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "agents.sendMessage", []byte(`{"sessionId":"s1","text":"hi"}`)); err == nil {
		t.Fatal("a 409 (ended session) must surface as an error, not silently fall back")
	}
	// The old fallback typed the text into the (dead) PTY — that must be gone.
	if n := len(rec.calls("/sessions/s1/input")); n != 0 {
		t.Fatalf("must not type into the PTY on 409, got %d input calls", n)
	}
}

func TestSendMessageNoFallbackOnSuccess(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "agents.sendMessage", []byte(`{"sessionId":"s1","text":"hi"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(rec.calls("/sessions/s1/input")); n != 0 {
		t.Fatalf("happy path must not type into the PTY, got %d input calls", n)
	}
}

// ── terminals.create / gate / resize ────────────────────────────────────────

func TestTerminalsCreateSpawnsShell(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	t.Setenv("SHELL", "/bin/bash")
	reg := newRegistry(newClaudemonClient(srv.URL))

	res, err := reg.handle(context.Background(), "terminals.create", []byte(`{"cwd":"/tmp"}`))
	if err != nil {
		t.Fatal(err)
	}
	spawn := rec.calls("/sessions/spawn")
	if len(spawn) != 1 {
		t.Fatalf("expected one spawn, got %d", len(spawn))
	}
	argv, _ := spawn[0].body["argv"].([]any)
	if len(argv) != 1 || argv[0] != "/bin/bash" {
		t.Errorf("expected argv [/bin/bash], got %v", argv)
	}
	if _, pinned := spawn[0].body["session_id"]; pinned {
		t.Error("a shell should not pin a session_id")
	}
	var out struct {
		SessionID string `json:"sessionId"`
	}
	if json.Unmarshal(res, &out); out.SessionID == "" {
		t.Error("expected a sessionId back")
	}
}

func TestGateForwards(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "claude.gate", []byte(`{"sessionId":"s1","on":true}`)); err != nil {
		t.Fatal(err)
	}
	g := rec.calls("/sessions/s1/gate")
	if len(g) != 1 || g[0].body["on"] != true {
		t.Fatalf("expected gate on=true, got %+v", g)
	}
}

func TestTerminalResizeForwards(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "sessions.terminalResize", []byte(`{"sessionId":"s1","cols":100,"rows":40}`)); err != nil {
		t.Fatal(err)
	}
	r := rec.calls("/sessions/s1/resize")
	if len(r) != 1 || r[0].body["cols"] != float64(100) || r[0].body["rows"] != float64(40) {
		t.Fatalf("expected resize 100x40, got %+v", r)
	}
}

// ── profiles CRUD round-trips through the same file the app reads ────────────

func TestProfilesCRUD(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)

	added, err := addProfile("Work", "~/work-cfg", []string{"--foo"}, []string{"mcp-1"})
	if err != nil {
		t.Fatal(err)
	}
	if added.ID == "" || !added.IsDefault {
		t.Fatalf("first profile should be default with an id, got %+v", added)
	}

	// Persisted to claude-profiles.json in the shape the app expects.
	raw, err := os.ReadFile(filepath.Join(dir, "workspacer", "claude-profiles.json"))
	if err != nil {
		t.Fatal(err)
	}
	var file struct {
		Profiles []profile `json:"profiles"`
	}
	if err := json.Unmarshal(raw, &file); err != nil || len(file.Profiles) != 1 {
		t.Fatalf("expected one persisted profile, got %s (err %v)", raw, err)
	}

	// Update name; isDefault stays.
	name := "Work2"
	updated, err := updateProfile(added.ID, profileUpdate{Name: &name})
	if err != nil || updated.Name != "Work2" {
		t.Fatalf("update failed: %+v err %v", updated, err)
	}

	// A second profile is not default.
	second, err := addProfile("Play", "", nil, nil)
	if err != nil || second.IsDefault {
		t.Fatalf("second profile should not be default: %+v err %v", second, err)
	}

	// Removing the default promotes the remaining profile.
	if err := removeProfile(added.ID); err != nil {
		t.Fatal(err)
	}
	left := readProfilesFile()
	if len(left) != 1 || left[0].ID != second.ID || !left[0].IsDefault {
		t.Fatalf("after removing default, the survivor should become default: %+v", left)
	}
}

// ── host fs ops ─────────────────────────────────────────────────────────────

func TestFsReadWriteRoundTrip(t *testing.T) {
	dir := t.TempDir()
	p := filepath.Join(dir, "note.txt")
	reg := newRegistry(newClaudemonClient("http://unused"))

	if _, err := reg.handle(context.Background(), "fs.write",
		json.RawMessage(`{"path":`+jsonStr(p)+`,"contents":"hello"}`)); err != nil {
		t.Fatal(err)
	}
	res, err := reg.handle(context.Background(), "fs.read", json.RawMessage(`{"path":`+jsonStr(p)+`}`))
	if err != nil {
		t.Fatal(err)
	}
	var got readFileResult
	if json.Unmarshal(res, &got); got.Contents != "hello" || got.Size != 5 {
		t.Fatalf("read back %+v, want contents hello size 5", got)
	}
}

func TestFsListDirReturnsDirsOnly(t *testing.T) {
	dir := t.TempDir()
	_ = os.Mkdir(filepath.Join(dir, "visible"), 0o755)
	_ = os.Mkdir(filepath.Join(dir, ".hidden"), 0o755)
	_ = os.WriteFile(filepath.Join(dir, "file.txt"), []byte("x"), 0o644)

	res, err := listHostDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(res.Dirs) != 1 || res.Dirs[0] != "visible" {
		t.Fatalf("expected only [visible], got %v", res.Dirs)
	}
	if res.Home == "" || res.Parent == "" {
		t.Fatalf("expected home+parent populated, got %+v", res)
	}
}

func jsonStr(s string) string {
	b, _ := json.Marshal(s)
	return string(b)
}
