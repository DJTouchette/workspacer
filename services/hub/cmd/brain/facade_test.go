package main

import (
	"context"
	"encoding/json"
	"net/url"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

func TestSpawnManagedInjectsWorkspacerFacade(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)
	reg.mcpFacadeURL = "http://127.0.0.1:7897/mcp"

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"codex","transport":"stream","cwd":"/tmp/proj","toolScope":"view","pluginTools":["jira","jira"]}`)); err != nil {
		t.Fatal(err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one managed spawn, got %d", len(managed))
	}
	body := managed[0].body
	mcpURL, _ := body["mcp"].(string)
	token := tokenFromFacadeURL(t, mcpURL)
	if !strings.HasPrefix(mcpURL, reg.mcpFacadeURL+"?") {
		t.Fatalf("mcp facade URL = %q, want tokenized %q", mcpURL, reg.mcpFacadeURL)
	}
	instructions, _ := body["instructions"].(string)
	sessionID, _ := body["session_id"].(string)
	if sessionID == "" || !strings.Contains(instructions, sessionID) || !strings.Contains(instructions, "view") {
		t.Fatalf("managed facade instructions should name the session and scope, got %q for %q", instructions, sessionID)
	}

	recToken := loadSessionToken(t, sessionID)
	if recToken.Token != token {
		t.Fatalf("token in facade URL does not match stored session token")
	}
	if recToken.Scope != authtoken.ScopeView {
		t.Fatalf("session token scope = %q, want view", recToken.Scope)
	}
	if !reflect.DeepEqual(recToken.Plugins, []string{"jira"}) {
		t.Fatalf("plugin grants = %v, want [jira]", recToken.Plugins)
	}
}

func TestSpawnClaudePTYInjectsFacadeMCPConfig(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)
	reg.mcpFacadeURL = "http://127.0.0.1:7898/mcp"

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"transport":"pty","cwd":"/tmp/proj","mcpFacade":true}`)); err != nil {
		t.Fatal(err)
	}
	spawns := rec.calls("/sessions/spawn")
	if len(spawns) != 1 {
		t.Fatalf("expected one PTY spawn, got %d", len(spawns))
	}
	sessionID, _ := spawns[0].body["session_id"].(string)
	argv := stringSlice(t, spawns[0].body["argv"])
	if !containsPair(argv, "--allowedTools", "mcp__workspacer") {
		t.Fatalf("facade spawn must pre-allow the workspacer MCP server, argv=%v", argv)
	}
	if !containsStr(argv, "--append-system-prompt") {
		t.Fatalf("facade spawn must append role instructions, argv=%v", argv)
	}
	cfgPath := valueAfterArg(t, argv, "--mcp-config")
	raw, err := os.ReadFile(cfgPath)
	if err != nil {
		t.Fatal(err)
	}
	var cfg claudeMCPConfig
	if err := json.Unmarshal(raw, &cfg); err != nil {
		t.Fatal(err)
	}
	entry, ok := cfg.MCPServers["workspacer"]
	if !ok {
		t.Fatalf("mcp config missing workspacer server: %s", raw)
	}
	if entry.URL != reg.mcpFacadeURL {
		t.Fatalf("mcp config URL = %q, want %q", entry.URL, reg.mcpFacadeURL)
	}
	recToken := loadSessionToken(t, sessionID)
	if got := entry.Headers["Authorization"]; got != "Bearer "+recToken.Token {
		t.Fatalf("Authorization header = %q, want stored session token", got)
	}
	if strings.Contains(strings.Join(argv, "\x00"), recToken.Token) {
		t.Fatalf("session token leaked into argv: %v", argv)
	}
}

func TestFleetHeadlessSpawnsReceiveWorkerEscalationContract(t *testing.T) {
	for _, provider := range []string{"codex", "opencode"} {
		t.Run(provider+" managed", func(t *testing.T) {
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			reg := newSpawnTestRegistry(t, srv.URL)
			if _, err := reg.handle(context.Background(), "agents.spawn",
				[]byte(`{"provider":"`+provider+`","cwd":"/tmp/proj","parentSessionId":"manager-1"}`)); err != nil {
				t.Fatal(err)
			}
			calls := rec.calls("/sessions/spawn-managed")
			if len(calls) != 1 {
				t.Fatalf("managed spawn calls = %d, want 1", len(calls))
			}
			instructions, _ := calls[0].body["instructions"].(string)
			if !strings.Contains(instructions, "wks-escalation") || !strings.Contains(instructions, "requiredAuthorityOrDecision") {
				t.Fatalf("plain %s worker missed escalation contract: %q", provider, instructions)
			}
		})
	}

	t.Run("claude pty without facade", func(t *testing.T) {
		rec := newRecorder()
		srv := rec.server()
		defer srv.Close()
		reg := newSpawnTestRegistry(t, srv.URL)
		if _, err := reg.handle(context.Background(), "agents.spawn",
			[]byte(`{"transport":"pty","cwd":"/tmp/proj","parentSessionId":"manager-1"}`)); err != nil {
			t.Fatal(err)
		}
		calls := rec.calls("/sessions/spawn")
		argv := stringSlice(t, calls[0].body["argv"])
		prompt := valueAfterArg(t, argv, "--append-system-prompt")
		if !strings.Contains(prompt, "wks-escalation") {
			t.Fatalf("plain PTY worker missed escalation contract: %v", argv)
		}
	})
}

func TestHeadlessFleetContractExcludesOrdinaryPanesAndManagers(t *testing.T) {
	for _, tc := range []struct {
		name, params string
	}{
		{"ordinary pane", `{"provider":"codex","cwd":"/tmp/proj"}`},
		{"fleet manager with accidental parent", `{"provider":"codex","cwd":"/tmp/proj","manager":true,"parentSessionId":"manager-0"}`},
	} {
		t.Run(tc.name, func(t *testing.T) {
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			reg := newSpawnTestRegistry(t, srv.URL)
			if _, err := reg.handle(context.Background(), "agents.spawn", []byte(tc.params)); err != nil {
				t.Fatal(err)
			}
			instructions, _ := rec.calls("/sessions/spawn-managed")[0].body["instructions"].(string)
			if strings.Contains(instructions, "wks-escalation") {
				t.Fatalf("non-worker received fleet escalation contract: %q", instructions)
			}
		})
	}
}

func TestSpawnFacadeRequestFailsWhenBrainHasNoFacadeURL(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	_, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"codex","cwd":"/tmp/proj","toolScope":"view"}`))
	if err == nil || !strings.Contains(err.Error(), "--mcp-facade") {
		t.Fatalf("expected missing facade URL error, got %v", err)
	}
	if len(rec.calls("/sessions/spawn")) != 0 || len(rec.calls("/sessions/spawn-managed")) != 0 {
		t.Fatalf("facade request without --mcp-facade must not reach claudemon: %+v", rec.hits)
	}
}

func TestSpawnManagerFacadeTokenUsesLocalConfigGrants(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	dir := tempConfigHome(t)
	t.Setenv("PATH", "")
	t.Setenv("WKS_CLAUDE_BIN", "")
	if err := os.MkdirAll(filepath.Join(dir, "workspacer"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(dir, "workspacer", "config.yaml"), []byte("agents:\n  fleetFullAccess: true\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := saveProfiles([]profile{
		{ID: "default", Name: "Default", IsDefault: true},
		{ID: "ops", Name: "Ops"},
	}); err != nil {
		t.Fatal(err)
	}
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.mcpFacadeURL = "http://127.0.0.1:7897/mcp"

	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"opencode","cwd":"/tmp/proj","manager":true,"toolScope":"operator"}`)); err != nil {
		t.Fatal(err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one managed spawn, got %d", len(managed))
	}
	sessionID, _ := managed[0].body["session_id"].(string)
	recToken := loadSessionToken(t, sessionID)
	if recToken.Role != "manager" {
		t.Fatalf("token role = %q, want manager", recToken.Role)
	}
	if !recToken.YoloAllowed {
		t.Fatalf("manager token should carry config-resolved yoloAllowed")
	}
	if !reflect.DeepEqual(recToken.ProfilesAllowed, []string{"default", "ops"}) {
		t.Fatalf("profilesAllowed = %v, want [default ops]", recToken.ProfilesAllowed)
	}
	if managed[0].body["yolo"] != false {
		t.Fatalf("session process yolo must still be controlled by the hub stamp, got %+v", managed[0].body)
	}
}

func loadSessionToken(t *testing.T, sessionID string) authtoken.Record {
	t.Helper()
	records, err := authtoken.Load(authtoken.DefaultPath())
	if err != nil {
		t.Fatal(err)
	}
	label := sessionFacadeTokenLabelPrefix + sessionID
	for _, rec := range records {
		if rec.Label == label {
			return rec
		}
	}
	t.Fatalf("no session token labeled %q in %v", label, records)
	return authtoken.Record{}
}

func tokenFromFacadeURL(t *testing.T, raw string) string {
	t.Helper()
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatal(err)
	}
	token := u.Query().Get("t")
	if token == "" {
		t.Fatalf("facade URL %q missing t query token", raw)
	}
	return token
}

func stringSlice(t *testing.T, raw any) []string {
	t.Helper()
	values, ok := raw.([]any)
	if !ok {
		t.Fatalf("expected []any, got %T", raw)
	}
	out := make([]string, 0, len(values))
	for _, v := range values {
		s, ok := v.(string)
		if !ok {
			t.Fatalf("expected string argv value, got %T", v)
		}
		out = append(out, s)
	}
	return out
}

func valueAfterArg(t *testing.T, argv []string, key string) string {
	t.Helper()
	for i := 0; i+1 < len(argv); i++ {
		if argv[i] == key {
			return argv[i+1]
		}
	}
	t.Fatalf("%s missing from argv: %v", key, argv)
	return ""
}
