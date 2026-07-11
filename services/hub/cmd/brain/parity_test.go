package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
	"strings"
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
		if r.URL.Path == "/sessions/spawn" || r.URL.Path == "/sessions/spawn-managed" {
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

// ── agents.spawn param-surface drift guard ──────────────────────────────────

// spawnParamsDeclined lists desktop agents.spawn params the brain deliberately
// does NOT mirror, with the reason. A param here must still exist on the
// desktop side (prune the entry when it's removed there); a desktop param that
// is neither in spawnParams' JSON tags nor here fails the drift guard below —
// mirror it or decline it explicitly.
var spawnParamsDeclined = map[string]string{
	"mcpFacade":  "the workspacer MCP facade server runs inside the desktop app; headless there is no facade URL to wire",
	"mcpItemIds": "per-spawn Library MCP servers need buildSessionMcpConfig (a desktop-owned session-scoped --mcp-config writer)",
}

// desktopSpawnParamRe pulls the field names out of the agents.spawn params type
// literal in hubCapabilities.ts (`provider?: AgentProvider;` → provider).
var desktopSpawnParamRe = regexp.MustCompile(`(?m)^\s*(\w+)\?:`)

// TestSpawnParamSurfaceMatchesDesktop cross-checks the desktop's agents.spawn
// param list (parsed from hubCapabilities.ts) against the brain's spawnParams
// JSON tags, so a param added on the desktop side fails here until the brain
// mirrors it (or documents why not in spawnParamsDeclined). The behavioural
// counterpart of capspec_guard_test's method-name cross-check. Skips (not
// fails) when the TS source isn't reachable (e.g. a hub-only checkout).
func TestSpawnParamSurfaceMatchesDesktop(t *testing.T) {
	// cmd/brain → repo root is four levels up (services/hub/cmd/brain).
	src := filepath.Join("..", "..", "..", "..", "apps", "desktop", "src", "main", "services", "hubCapabilities.ts")
	data, err := os.ReadFile(src)
	if err != nil {
		t.Skipf("hubCapabilities.ts not reachable (%v); skipping cross-repo cross-check", err)
	}
	text := string(data)
	// Isolate the agents.spawn registration's destructure type literal.
	start := strings.Index(text, "registerCapability('agents.spawn'")
	if start < 0 {
		t.Fatal("hubCapabilities.ts no longer registers 'agents.spawn' — update this guard")
	}
	text = text[start:]
	open := strings.Index(text, "} = (params ?? {}) as {")
	end := strings.Index(text[open+1:], "};")
	if open < 0 || end < 0 {
		t.Fatal("could not find the agents.spawn params type literal — the destructuring syntax changed; update this guard")
	}
	block := text[open : open+1+end]

	desktop := map[string]bool{}
	for _, m := range desktopSpawnParamRe.FindAllStringSubmatch(block, -1) {
		desktop[m[1]] = true
	}
	if len(desktop) < 5 {
		t.Fatalf("parsed implausibly few desktop spawn params (%v) — the regex stopped matching", desktop)
	}

	brain := map[string]bool{}
	tp := reflect.TypeOf(spawnParams{})
	for i := 0; i < tp.NumField(); i++ {
		if tag := strings.Split(tp.Field(i).Tag.Get("json"), ",")[0]; tag != "" {
			brain[tag] = true
		}
	}

	for param := range desktop {
		if !brain[param] && spawnParamsDeclined[param] == "" {
			t.Errorf("desktop agents.spawn takes %q but the brain's spawnParams doesn't — mirror it or add it to spawnParamsDeclined with a reason", param)
		}
	}
	for param := range brain {
		if !desktop[param] {
			t.Errorf("brain spawnParams has %q but the desktop's agents.spawn doesn't — the surfaces must stay identical", param)
		}
	}
	for param := range spawnParamsDeclined {
		if !desktop[param] {
			t.Errorf("spawnParamsDeclined lists %q but the desktop no longer takes it — prune the entry", param)
		}
		if brain[param] {
			t.Errorf("%q is both mirrored in spawnParams and declined in spawnParamsDeclined — drop one", param)
		}
	}
}

// ── snapshot field-shape drift guard ─────────────────────────────────────────

// snapshotFieldsRequired lists the desktop-snapshot (camelCase) fields the bus
// clients read off a session row — mobile.html directly, the web renderer via
// webBackend — that compatSnapshot must therefore emit (or pass through) on
// every brain-served row. Adding a snapshot read to mobile.html without
// teaching the overlay fails here; pruning a field there should prune it here.
var snapshotFieldsRequired = []string{
	"sessionId",
	"status",
	"ambientState",
	"lastActivity",
	"cwd",
	"transport",
	"usage",
	"pendingApproval",
	"pendingQuestions",
}

// snapshotFieldsDeclined lists desktop-snapshot fields the clients read that
// the brain deliberately does NOT provide, with the reason. Entries must still
// be read by mobile.html (prune when the client stops using them).
var snapshotFieldsDeclined = map[string]string{
	"conversation": "turn-by-turn transcript lives in claudemon's /conversation endpoint, not the session row; folding it into every snapshot/publish would ship whole transcripts per state tick",
	"liveCwd":      "statusline-derived live cwd is a desktop enrichment; clients fall back to cwd (mobile agentName: liveCwd || cwd)",
}

// TestCompatSnapshotCoversMobileFields cross-checks the field names the mobile
// client reads against the compat overlay — the wire-shape counterpart of the
// spawn-param guard above. Skips when mobile.html isn't reachable.
func TestCompatSnapshotCoversMobileFields(t *testing.T) {
	src := filepath.Join("..", "hub", "mobile.html")
	data, err := os.ReadFile(src)
	if err != nil {
		t.Skipf("mobile.html not reachable (%v); skipping cross-check", err)
	}
	mobile := string(data)

	// A representative claudemon row exercising every mapped branch.
	row := json.RawMessage(`{
		"session_id":"s1","mode":"responding","cwd":"/tmp","provider":"claude",
		"transport":"stream","archived":false,"updated_at":"2026-07-10T12:00:00Z",
		"usage":{"model":"m","context_tokens":1,"context_limit":2,"cost_usd":0.1},
		"pending":null}`)
	var m map[string]any
	if err := json.Unmarshal(compatSnapshot(row), &m); err != nil {
		t.Fatal(err)
	}

	for _, field := range snapshotFieldsRequired {
		if !strings.Contains(mobile, field) {
			t.Errorf("snapshotFieldsRequired lists %q but mobile.html no longer references it — prune the entry", field)
		}
		if _, ok := m[field]; !ok {
			t.Errorf("mobile reads snapshot field %q but compatSnapshot doesn't emit it — map it or decline it with a reason", field)
		}
	}
	for field, reason := range snapshotFieldsDeclined {
		if reason == "" {
			t.Errorf("snapshotFieldsDeclined[%q] needs a reason", field)
		}
		if !strings.Contains(mobile, field) {
			t.Errorf("snapshotFieldsDeclined lists %q but mobile.html no longer references it — prune the entry", field)
		}
	}
	// usage sub-shape: mobile reads u.contextTokens / contextLimit / costUSD /
	// model (ctxPct, fleetCard, shortModel).
	u, _ := m["usage"].(map[string]any)
	for _, k := range []string{"model", "contextTokens", "contextLimit", "costUSD"} {
		if _, ok := u[k]; !ok {
			t.Errorf("usage overlay missing %q (mobile ctxPct/fleetCard read it)", k)
		}
	}
}
