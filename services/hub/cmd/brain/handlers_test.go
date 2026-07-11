package main

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// These tests stand up a fake claudemon with httptest and verify the handlers
// forward the right requests — real verification of the routing/payloads without
// a live daemon or hub.

// newSpawnTestRegistry builds a registry whose config/bin resolution can't leak
// in from the developer's machine: an isolated config dir (so the transport
// default is the shipped one, not the user's config.yaml) and an empty PATH (so
// managed bins resolve to the bare provider name).
func newSpawnTestRegistry(t *testing.T, srvURL string) *registry {
	t.Helper()
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("PATH", "")
	t.Setenv("WKS_CLAUDE_BIN", "")
	return newRegistry(newClaudemonClient(srvURL))
}

func TestSpawnForwardsArgvAndReturnsSessionID(t *testing.T) {
	var gotPath string
	var gotBody spawnReq
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath = r.URL.Path
		_ = json.NewDecoder(r.Body).Decode(&gotBody)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": gotBody.SessionID})
	}))
	defer srv.Close()

	reg := newSpawnTestRegistry(t, srv.URL)
	// transport:pty is explicit now that the config default is 'stream' (this
	// test exercises the classic PTY argv spawn).
	params := []byte(`{"cwd":"/tmp/proj/","skipPermissions":true,"transport":"pty"}`)
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
	// SECURITY: agents.spawn is the remote/bus path — a requested bypass is
	// forced off (mirrors hubCapabilities.ts), so the flag must NOT ride.
	if containsStr(gotBody.Argv, "--dangerously-skip-permissions") {
		t.Errorf("bus spawns must never auto-bypass approvals, got argv %v", gotBody.Argv)
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

// ── agents.spawn provider/transport dispatch (parity with hubCapabilities.ts) ─

// TestSpawnDefaultProviderIsClaudeStream: no provider AND no transport resolves
// to claude on the STREAM transport — matching the desktop default
// (config_defaults.json claude.transport=stream, the single source of truth both
// runtimes share). It routes through spawn-managed (claude-stream), not the
// classic PTY argv spawn. Regression guard for the old drift, where the brain's
// missing default made a headless/web spawn come up PTY while the desktop's
// same spawn came up stream.
func TestSpawnDefaultProviderIsClaudeStream(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	if _, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"cwd":"/tmp"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(rec.calls("/sessions/spawn")); n != 0 {
		t.Fatalf("stream default must not hit the PTY spawn endpoint, got %d calls", n)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one claude-stream spawn-managed call, got %d", len(managed))
	}
	if managed[0].body["provider"] != "claude" {
		t.Errorf("default provider must be claude, got %v", managed[0].body["provider"])
	}
}

// TestSpawnCodexStreamForwardsManagedPayload: codex + transport 'stream' POSTs
// spawn-managed with the snake_case wire shape — transport:"stream" must ride
// (the headless-codex field the desktop once dropped), plus model/effort/cwd
// and a pinned fresh session id. yolo stays false.
func TestSpawnCodexStreamForwardsManagedPayload(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	res, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"codex","transport":"stream","cwd":"/tmp/proj/","model":"gpt-5.3-codex","effort":"high"}`))
	if err != nil {
		t.Fatal(err)
	}
	if n := len(rec.calls("/sessions/spawn")); n != 0 {
		t.Fatalf("codex must not spawn a claude PTY, got %d /sessions/spawn calls", n)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one spawn-managed call, got %d", len(managed))
	}
	body := managed[0].body
	if body["provider"] != "codex" || body["transport"] != "stream" {
		t.Errorf("expected provider codex transport stream on the wire, got %+v", body)
	}
	if body["model"] != "gpt-5.3-codex" || body["effort"] != "high" || body["cwd"] != "/tmp/proj" {
		t.Errorf("model/effort/cwd wrong: %+v", body)
	}
	if body["yolo"] != false {
		t.Errorf("bus spawns must send yolo=false, got %v", body["yolo"])
	}
	id, _ := body["session_id"].(string)
	if id == "" {
		t.Error("a fresh managed spawn must pin a session_id")
	}
	var out struct {
		SessionID string `json:"sessionId"`
	}
	if json.Unmarshal(res, &out); out.SessionID != id {
		t.Errorf("result sessionId %q != pinned id %q", out.SessionID, id)
	}
}

// TestSpawnClaudeStreamBranch: claude + transport 'stream' is managed too — but
// deliberately carries NO wire `transport` key (spawn-managed claude IS the
// stream adapter); permission_mode and resume ride, and the resumed id doubles
// as the pinned session_id (managed ids are not re-pinnable).
func TestSpawnClaudeStreamBranch(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	_, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"claude","transport":"stream","cwd":"/tmp/x","permissionMode":"acceptEdits","resumeSessionId":"abc-123"}`))
	if err != nil {
		t.Fatal(err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one spawn-managed call, got %d", len(managed))
	}
	body := managed[0].body
	if body["provider"] != "claude" {
		t.Errorf("provider = %v, want claude", body["provider"])
	}
	if _, hasTransport := body["transport"]; hasTransport {
		t.Errorf("claude-stream must not send a wire transport key, got %v", body["transport"])
	}
	if body["permission_mode"] != "acceptEdits" {
		t.Errorf("permission_mode = %v, want acceptEdits", body["permission_mode"])
	}
	if body["resume"] != "abc-123" || body["session_id"] != "abc-123" {
		t.Errorf("resume must ride and reuse the prior id as session_id, got %+v", body)
	}
}

// TestSpawnClaudeStreamDefaultsPermissionMode: an omitted mode resolves to
// 'default' on the wire, matching the desktop's managedSpawn resolution.
func TestSpawnClaudeStreamDefaultsPermissionMode(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"transport":"stream","cwd":"/tmp"}`)); err != nil {
		t.Fatal(err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 || managed[0].body["permission_mode"] != "default" {
		t.Fatalf("expected permission_mode default, got %+v", managed)
	}
}

// TestSpawnClaudeTransportConfigDefault: with no explicit transport, the
// config's claude.transport decides — mirroring the desktop's
// `reqTransport ?? config.claude.transport ?? 'pty'`.
func TestSpawnClaudeTransportConfigDefault(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	t.Setenv("PATH", "")
	t.Setenv("WKS_CLAUDE_BIN", "")
	if err := os.MkdirAll(filepath.Join(dir, "workspacer"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "workspacer", "config.yaml"),
		[]byte("claude:\n  transport: stream\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	reg := newRegistry(newClaudemonClient(srv.URL))

	if _, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"cwd":"/tmp"}`)); err != nil {
		t.Fatal(err)
	}
	if len(rec.calls("/sessions/spawn-managed")) != 1 || len(rec.calls("/sessions/spawn")) != 0 {
		t.Fatalf("config claude.transport=stream must route through spawn-managed, got %+v", rec.hits)
	}
	// And an explicit 'pty' overrides the config default back to the PTY path.
	if _, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"cwd":"/tmp","transport":"pty"}`)); err != nil {
		t.Fatal(err)
	}
	if len(rec.calls("/sessions/spawn")) != 1 {
		t.Fatalf("explicit transport pty must win over the config default, got %+v", rec.hits)
	}
}

// TestSpawnManagedResume: a codex resume rides the wire `resume` field and
// reuses the prior id as session_id; opencode/pi have no resume on the wire
// (matching the desktop dispatch) but still pin the id.
func TestSpawnManagedResume(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"codex","cwd":"/tmp","resumeSessionId":"prior-1"}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"opencode","cwd":"/tmp","resumeSessionId":"prior-2"}`)); err != nil {
		t.Fatal(err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 2 {
		t.Fatalf("expected two spawn-managed calls, got %d", len(managed))
	}
	codex := managed[0].body
	if codex["resume"] != "prior-1" || codex["session_id"] != "prior-1" {
		t.Errorf("codex resume must ride resume + session_id, got %+v", codex)
	}
	// A codex resume stays on the default hybrid unless stream is asked for.
	if _, hasTransport := codex["transport"]; hasTransport {
		t.Errorf("codex resume without transport param must not send one, got %v", codex["transport"])
	}
	oc := managed[1].body
	if _, hasResume := oc["resume"]; hasResume {
		t.Errorf("opencode must not send a wire resume (desktop parity), got %v", oc["resume"])
	}
	if oc["session_id"] != "prior-2" {
		t.Errorf("opencode resume must still pin the prior id, got %+v", oc)
	}
}

// TestSpawnRemoteBypassForcedOff: the security rule — a bus caller may never
// auto-bypass approvals, whatever the provider or spelling.
func TestSpawnRemoteBypassForcedOff(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	// Managed provider: skipPermissions requested → yolo forced false.
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"codex","cwd":"/tmp","skipPermissions":true}`)); err != nil {
		t.Fatal(err)
	}
	// Claude stream: 'bypassPermissions' requested → clamped to 'default'.
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"transport":"stream","cwd":"/tmp","permissionMode":"bypassPermissions"}`)); err != nil {
		t.Fatal(err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 2 {
		t.Fatalf("expected two spawn-managed calls, got %d", len(managed))
	}
	if managed[0].body["yolo"] != false {
		t.Errorf("codex: remote skipPermissions must be forced off, got yolo=%v", managed[0].body["yolo"])
	}
	if managed[1].body["yolo"] != false || managed[1].body["permission_mode"] != "default" {
		t.Errorf("claude stream: remote bypass mode must be clamped, got %+v", managed[1].body)
	}
	// Claude PTY (transport:pty explicit now the default is stream): 'yolo'
	// spelling clamps too; a passthrough mode still rides.
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","skipPermissions":true,"permissionMode":"yolo","transport":"pty"}`)); err != nil {
		t.Fatal(err)
	}
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","permissionMode":"plan","transport":"pty"}`)); err != nil {
		t.Fatal(err)
	}
	spawn := rec.calls("/sessions/spawn")
	if len(spawn) != 2 {
		t.Fatalf("expected two PTY spawns, got %d", len(spawn))
	}
	argv := argvStrings(spawn[0].body["argv"])
	if containsStr(argv, "--dangerously-skip-permissions") || containsStr(argv, "--permission-mode") {
		t.Errorf("PTY: remote bypass must be stripped from argv, got %v", argv)
	}
	argv = argvStrings(spawn[1].body["argv"])
	if !containsPair(argv, "--permission-mode", "plan") {
		t.Errorf("PTY: a non-bypass permissionMode must pass through, got %v", argv)
	}
}

// TestSpawnClaudePTYResumeStillWorks: the pre-existing PTY resume contract
// (--resume <id>, no --session-id) survives the provider/transport dispatch.
func TestSpawnClaudePTYResumeStillWorks(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","resumeSessionId":"abc-123","transport":"pty"}`)); err != nil {
		t.Fatal(err)
	}
	spawn := rec.calls("/sessions/spawn")
	if len(spawn) != 1 {
		t.Fatalf("expected one PTY spawn, got %d", len(spawn))
	}
	argv := argvStrings(spawn[0].body["argv"])
	if !containsPair(argv, "--resume", "abc-123") || containsStr(argv, "--session-id") {
		t.Errorf("PTY resume must ride --resume without --session-id, got %v", argv)
	}
}

// argvStrings coerces a recorded JSON argv ([]any) back to []string.
func argvStrings(v any) []string {
	raw, _ := v.([]any)
	out := make([]string, 0, len(raw))
	for _, a := range raw {
		if s, ok := a.(string); ok {
			out = append(out, s)
		}
	}
	return out
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
