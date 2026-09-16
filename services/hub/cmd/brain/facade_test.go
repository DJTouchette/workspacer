package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

func TestHeadlessFacadeHealthIsCheckedForEverySpawnBeforeMinting(t *testing.T) {
	rec := newRecorder()
	claudemon := rec.server()
	defer claudemon.Close()
	healthy := true
	const hubURL = "ws://127.0.0.1:7895/bus"
	var facade *httptest.Server
	facade = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		u, _ := url.Parse(facade.URL)
		_ = json.NewEncoder(w).Encode(map[string]any{
			"status": "ok", "service": "workspacer-mcp-facade",
			"hubConnected": healthy, "pluginCatalogReady": healthy,
			"listenAddr": u.Host, "hubUrl": hubURL,
		})
	}))
	defer facade.Close()

	reg := newRegistry(newClaudemonClient(claudemon.URL))
	reg.mcpFacadeURL = facade.URL + "/mcp"
	reg.mcpFacadeHubURL = hubURL
	before := len(mustLoadTokens(t))

	if _, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"provider":"codex","cwd":"/tmp/proj"}`)); err != nil {
		t.Fatal(err)
	}
	first := rec.calls("/sessions/spawn-managed")[0].body
	if _, ok := first["mcp"]; !ok {
		t.Fatalf("healthy facade was not injected: %+v", first)
	}
	if got := len(mustLoadTokens(t)); got != before+1 {
		t.Fatalf("healthy spawn token count = %d, want %d", got, before+1)
	}

	healthy = false
	if _, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"provider":"codex","cwd":"/tmp/proj"}`)); err != nil {
		t.Fatal(err)
	}
	second := rec.calls("/sessions/spawn-managed")[1].body
	if _, ok := second["mcp"]; ok {
		t.Fatalf("down facade URL was injected: %+v", second)
	}
	if got := len(mustLoadTokens(t)); got != before+1 {
		t.Fatalf("unhealthy spawn minted a token: got %d, want %d", got, before+1)
	}

	healthy = true
	if _, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"provider":"codex","cwd":"/tmp/proj"}`)); err != nil {
		t.Fatal(err)
	}
	third := rec.calls("/sessions/spawn-managed")[2].body
	if _, ok := third["mcp"]; !ok {
		t.Fatalf("recovered facade was not injected: %+v", third)
	}
	if got := len(mustLoadTokens(t)); got != before+2 {
		t.Fatalf("recovered spawn token count = %d, want %d", got, before+2)
	}
}

func TestStandaloneWrongOrDisconnectedFacadeMintsNoToken(t *testing.T) {
	for _, tc := range []struct {
		name string
		body map[string]any
	}{
		{"wrong service", map[string]any{"status": "ok", "service": "not-workspacer", "hubConnected": true, "pluginCatalogReady": true}},
		{"disconnected", map[string]any{"status": "ok", "service": "workspacer-mcp-facade", "hubConnected": false, "pluginCatalogReady": true}},
		{"catalog not ready", map[string]any{"status": "ok", "service": "workspacer-mcp-facade", "hubConnected": true, "pluginCatalogReady": false}},
		{"wrong bind", map[string]any{"status": "ok", "service": "workspacer-mcp-facade", "hubConnected": true, "pluginCatalogReady": true, "listenAddr": "127.0.0.1:1"}},
		{"wrong hub", map[string]any{"status": "ok", "service": "workspacer-mcp-facade", "hubConnected": true, "pluginCatalogReady": true, "hubUrl": "ws://127.0.0.1:1/bus"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			rec := newRecorder()
			claudemon := rec.server()
			defer claudemon.Close()
			var facade *httptest.Server
			facade = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				u, _ := url.Parse(facade.URL)
				body := map[string]any{"listenAddr": u.Host, "hubUrl": "ws://127.0.0.1:7895/bus"}
				for k, v := range tc.body {
					body[k] = v
				}
				_ = json.NewEncoder(w).Encode(body)
			}))
			defer facade.Close()
			reg := newRegistry(newClaudemonClient(claudemon.URL))
			reg.mcpFacadeURL = facade.URL + "/mcp"
			reg.mcpFacadeHubURL = "ws://127.0.0.1:7895/bus"
			before := len(mustLoadTokens(t))
			if _, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"provider":"codex","cwd":"/tmp/proj"}`)); err != nil {
				t.Fatal(err)
			}
			if _, ok := rec.calls("/sessions/spawn-managed")[0].body["mcp"]; ok {
				t.Fatal("unverified facade was injected")
			}
			if got := len(mustLoadTokens(t)); got != before {
				t.Fatalf("unverified facade minted token: %d -> %d", before, got)
			}
		})
	}
}

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
	if sessionID == "" || !strings.Contains(instructions, sessionID) || !strings.Contains(instructions, "operator") {
		t.Fatalf("managed facade instructions should name the session and scope, got %q for %q", instructions, sessionID)
	}

	recToken := loadSessionToken(t, sessionID)
	if recToken.Token != token {
		t.Fatalf("token in facade URL does not match stored session token")
	}
	if recToken.Scope != authtoken.ScopeOperator {
		t.Fatalf("session token scope = %q, want operator", recToken.Scope)
	}
	if len(recToken.Plugins) != 1 || recToken.Plugins[0] != "*" {
		t.Fatalf("plugin compatibility field = %v, want ambient wildcard", recToken.Plugins)
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
			reg.mcpFacadeURL = "http://127.0.0.1:7897/mcp"
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

func TestFleetClaudePTYComposesProfilePromptAndEscalationContract(t *testing.T) {
	for _, tc := range []struct {
		name string
		args []string
	}{
		{"split profile arg", []string{"--append-system-prompt", "PROFILE SPLIT"}},
		{"equals profile arg", []string{"--append-system-prompt=PROFILE EQUALS"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			rec := newRecorder()
			srv := rec.server()
			defer srv.Close()
			reg := newSpawnTestRegistry(t, srv.URL)
			if err := saveProfiles([]profile{{ID: "profile", Name: "Profile", IsDefault: true, ExtraArgs: tc.args}}); err != nil {
				t.Fatal(err)
			}

			if _, err := reg.handle(context.Background(), "agents.spawn",
				[]byte(`{"transport":"pty","cwd":"/tmp/proj","parentSessionId":"manager-1","profileId":"profile"}`)); err != nil {
				t.Fatal(err)
			}
			argv := stringSlice(t, rec.calls("/sessions/spawn")[0].body["argv"])
			count := 0
			for _, arg := range argv {
				if arg == "--append-system-prompt" || strings.HasPrefix(arg, "--append-system-prompt=") {
					count++
				}
			}
			if count != 1 {
				t.Fatalf("append-system-prompt count = %d, want one: %v", count, argv)
			}
			prompt := valueAfterArg(t, argv, "--append-system-prompt")
			if !strings.Contains(prompt, "PROFILE") || !strings.Contains(prompt, "wks-escalation") {
				t.Fatalf("combined prompt missed the profile or contract: %q", prompt)
			}
			if strings.Index(prompt, "PROFILE") > strings.Index(prompt, "wks-escalation") {
				t.Fatalf("profile prompt must precede host contract: %q", prompt)
			}
		})
	}
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
			reg.mcpFacadeURL = "http://127.0.0.1:7897/mcp"
			params := tc.params
			if tc.name == "ordinary pane" {
				params = fmt.Sprintf(`{"provider":"codex","cwd":%q}`, t.TempDir())
			}
			if _, err := reg.handle(context.Background(), "agents.spawn", []byte(params)); err != nil {
				t.Fatal(err)
			}
			instructions, _ := rec.calls("/sessions/spawn-managed")[0].body["instructions"].(string)
			if strings.Contains(instructions, "wks-escalation") {
				t.Fatalf("non-worker received fleet escalation contract: %q", instructions)
			}
			if tc.name == "ordinary pane" {
				if !strings.Contains(instructions, ".workspacer") || !strings.Contains(instructions, "spawn-agent/SKILL.md") || !strings.Contains(instructions, "project-brief/SKILL.md") {
					t.Fatalf("ordinary agent missed collaboration skills: %q", instructions)
				}
				if strings.Contains(instructions, "# Spawn an agent") || strings.Contains(instructions, "---\nname:") {
					t.Fatalf("ordinary prompt embedded skill bodies: %q", instructions)
				}
			} else if strings.Contains(instructions, "spawn-agent/SKILL.md") || strings.Contains(instructions, "project-brief/SKILL.md") {
				t.Fatalf("Fleet Manager received ordinary-agent skill doctrine: %q", instructions)
			}
		})
	}
}

func TestSpawnWithoutVerifiedFacadeURLDoesNotInventAnEndpoint(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	before, err := authtoken.Load(authtoken.DefaultPath())
	if err != nil {
		t.Fatal(err)
	}
	_, err = reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"provider":"codex","cwd":"/tmp/proj","toolScope":"view"}`))
	if err != nil {
		t.Fatalf("spawn without a verified facade should still launch honestly: %v", err)
	}
	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("spawn calls = %d, want 1", len(managed))
	}
	if _, advertised := managed[0].body["mcp"]; advertised {
		t.Fatalf("spawn advertised a dead MCP endpoint: %+v", managed[0].body)
	}
	after, err := authtoken.Load(authtoken.DefaultPath())
	if err != nil {
		t.Fatal(err)
	}
	if len(after) != len(before) {
		t.Fatalf("spawn without facade minted a dangling token: before=%d after=%d", len(before), len(after))
	}
}

func TestSpawnManagerFacadeTokenHasAmbientOperatorAuthorityWithoutLegacyGrants(t *testing.T) {
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
	reg.facadeHealthProbe = func(string, string) bool { return true }

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
	if recToken.YoloAllowed {
		t.Fatalf("manager token retained obsolete yoloAllowed grant")
	}
	if len(recToken.ProfilesAllowed) != 0 {
		t.Fatalf("manager token retained obsolete profilesAllowed grant: %v", recToken.ProfilesAllowed)
	}
	if managed[0].body["yolo"] != false {
		t.Fatalf("session process yolo must still be controlled by the hub stamp, got %+v", managed[0].body)
	}
}

func TestFailedHeadlessSpawnsRevokeMintedFacadeToken(t *testing.T) {
	for _, tc := range []struct {
		name, provider, endpoint string
	}{
		{"managed", "codex", "/sessions/spawn-managed"},
		{"pty", "claude", "/sessions/spawn"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			rec := newRecorder()
			rec.status[tc.endpoint] = 500
			srv := rec.server()
			defer srv.Close()
			reg := newSpawnTestRegistry(t, srv.URL)
			reg.mcpFacadeURL = "http://127.0.0.1:7897/mcp"
			transport := "stream"
			if tc.name == "pty" {
				transport = "pty"
			}
			_, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"provider":"`+tc.provider+`","transport":"`+transport+`","cwd":"/tmp/proj"}`))
			if err == nil {
				t.Fatal("daemon failure unexpectedly launched")
			}
			calls := rec.calls(tc.endpoint)
			if len(calls) != 1 {
				t.Fatalf("spawn calls = %d, want 1", len(calls))
			}
			sessionID, _ := calls[0].body["session_id"].(string)
			for _, token := range mustLoadTokens(t) {
				if token.Label == sessionFacadeTokenLabelPrefix+sessionID {
					t.Fatalf("failed spawn leaked facade token for %s", sessionID)
				}
			}
		})
	}
}

func TestHeadlessSessionEndRevokesFacadeTokenOncePerLifecycle(t *testing.T) {
	reg := newRegistry(newClaudemonClient("http://127.0.0.1:0"))
	rec, err := mintSessionFacadeToken("lifecycle", authtoken.ScopeOperator, []string{"*"}, nil, false, "")
	if err != nil {
		t.Fatal(err)
	}
	store := newSessionStore()
	revokes := 0
	lifecycleEdges := 0
	store.onEnd = func(string) { lifecycleEdges++ }
	store.onEndedRetry = func(id string) bool {
		revokes++
		if err := revokeSessionFacadeToken(id); err != nil {
			t.Errorf("revoke: %v", err)
			return false
		}
		return true
	}
	reg.store = store
	store.set("lifecycle", json.RawMessage(`{"session_id":"lifecycle","mode":"input"}`))
	store.set("lifecycle", json.RawMessage(`{"session_id":"lifecycle","mode":"stopped"}`))
	store.set("lifecycle", json.RawMessage(`{"session_id":"lifecycle","mode":"stopped"}`))
	if revokes != 1 {
		t.Fatalf("end revokes = %d, want exactly 1", revokes)
	}
	if lifecycleEdges != 1 {
		t.Fatalf("lifecycle end effects = %d, want exactly 1", lifecycleEdges)
	}
	for _, row := range mustLoadTokens(t) {
		if row.Token == rec.Token {
			t.Fatal("ended lifecycle retained facade token")
		}
	}
}

func TestHeadlessSessionEndRetriesTransientFacadeTokenSaveFailure(t *testing.T) {
	rec, err := mintSessionFacadeToken("retry-revoke", authtoken.ScopeOperator, []string{"*"}, nil, false, "")
	if err != nil {
		t.Fatal(err)
	}

	originalSave := saveSessionFacadeTokens
	t.Cleanup(func() { saveSessionFacadeTokens = originalSave })
	saveCalls := 0
	saveSessionFacadeTokens = func(path string, rows []authtoken.Record) error {
		saveCalls++
		if saveCalls == 1 {
			return errors.New("injected transient token-store failure")
		}
		return originalSave(path, rows)
	}

	store := newSessionStore()
	lifecycleEdges := 0
	revokeAttempts := 0
	store.onEnd = func(string) { lifecycleEdges++ }
	store.onEndedRetry = func(id string) bool {
		revokeAttempts++
		return revokeSessionFacadeToken(id) == nil
	}

	store.set("retry-revoke", json.RawMessage(`{"session_id":"retry-revoke","mode":"input"}`))
	store.set("retry-revoke", json.RawMessage(`{"session_id":"retry-revoke","mode":"stopped"}`))
	if lifecycleEdges != 1 || revokeAttempts != 1 || saveCalls != 1 {
		t.Fatalf("first stop: lifecycle=%d attempts=%d saves=%d, want 1/1/1", lifecycleEdges, revokeAttempts, saveCalls)
	}
	if got := loadSessionToken(t, "retry-revoke"); got.Token != rec.Token {
		t.Fatalf("failed revocation removed or replaced token: got %q want %q", got.Token, rec.Token)
	}

	// A duplicate stopped observation is not a new lifecycle edge, but it must
	// retry the failed persistent cleanup.
	store.set("retry-revoke", json.RawMessage(`{"session_id":"retry-revoke","mode":"stopped"}`))
	if lifecycleEdges != 1 || revokeAttempts != 2 || saveCalls != 2 {
		t.Fatalf("retry stop: lifecycle=%d attempts=%d saves=%d, want 1/2/2", lifecycleEdges, revokeAttempts, saveCalls)
	}
	for _, row := range mustLoadTokens(t) {
		if row.Token == rec.Token {
			t.Fatal("successful retry retained facade token")
		}
	}

	// Persistent success is remembered across an SSE reconnect/reseed; the
	// repeated ended row is neither a new lifecycle edge nor another revoke.
	store.seed(map[string]json.RawMessage{
		"retry-revoke": json.RawMessage(`{"session_id":"retry-revoke","mode":"stopped"}`),
	})
	if lifecycleEdges != 1 || revokeAttempts != 2 || saveCalls != 2 {
		t.Fatalf("post-success duplicate: lifecycle=%d attempts=%d saves=%d, want 1/2/2", lifecycleEdges, revokeAttempts, saveCalls)
	}
}

func mustLoadTokens(t *testing.T) []authtoken.Record {
	t.Helper()
	records, err := authtoken.Load(authtoken.DefaultPath())
	if err != nil {
		t.Fatal(err)
	}
	return records
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
