package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"testing"
)

// A MANAGER spawn that names no provider must land on the harness config says
// the role runs on — agents.managerProvider.
//
// The reported bug: Settings said the manager runs on codex and the session
// came up on Claude. The setting was read by ONE desktop launcher and by
// nothing in either backend, so every headless start — which is every manager
// in `workspacer serve`, plus anything launched from the phone, the web client
// or a hub job — arrived with `provider: ""` and fell through to claude. A
// silently-Claude manager is indistinguishable from a working one, which is
// why this is pinned on the wire rather than in the resolver alone.
//
// TWIN: apps/desktop/src/main/lib/roleProviders.test.ts.
func roleProviderRig(t *testing.T, cfg map[string]any) (*registry, func(string) []recordedCall) {
	t.Helper()
	tempConfigHome(t)
	if err := writeConfigYAML(cfg); err != nil {
		t.Fatalf("seed config: %v", err)
	}
	var calls []recordedCall
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var body map[string]any
		_ = json.NewDecoder(r.Body).Decode(&body)
		calls = append(calls, recordedCall{path: r.URL.Path, body: body})
		id, _ := body["session_id"].(string)
		_ = json.NewEncoder(w).Encode(map[string]string{"session_id": id})
	}))
	t.Cleanup(srv.Close)
	t.Setenv("PATH", "")
	t.Setenv("WKS_CLAUDE_BIN", "")
	reg := newRegistry(newClaudemonClient(srv.URL))
	reg.meta = newMetaStore()
	// A facade spawn needs the facade URL; without one the spawn is refused
	// before it ever reaches the provider split.
	reg.mcpFacadeURL = srv.URL + "/mcp"
	return reg, func(path string) []recordedCall {
		var out []recordedCall
		for _, c := range calls {
			if c.path == path {
				out = append(out, c)
			}
		}
		return out
	}
}

type recordedCall struct {
	path string
	body map[string]any
}

func TestRoleSpawnWithNoProviderTakesTheConfiguredHarness(t *testing.T) {
	for _, tc := range []struct {
		name   string
		cfg    map[string]any
		params string
		want   string
	}{
		{
			name:   "fleet manager",
			cfg:    map[string]any{"agents": map[string]any{"managerProvider": "codex"}},
			params: `{"cwd":"/tmp","manager":true}`,
			want:   "codex",
		},
		{
			// The third harness the manager picker offers. Headless is where
			// this resolver is the ONLY reader of the setting — there is no
			// renderer here to remember it — so a copilot manager started from
			// `workspacer serve`, the phone or a hub job hangs entirely on
			// "copilot" being in roleProviderDefault's accept list.
			name:   "fleet manager on copilot",
			cfg:    map[string]any{"agents": map[string]any{"managerProvider": "copilot"}},
			params: `{"cwd":"/tmp","manager":true}`,
			want:   "copilot",
		},
	} {
		t.Run(tc.name, func(t *testing.T) {
			reg, calls := roleProviderRig(t, tc.cfg)
			if _, err := reg.handle(context.Background(), "agents.spawn", []byte(tc.params)); err != nil {
				t.Fatal(err)
			}
			managed := calls("/sessions/spawn-managed")
			if len(managed) != 1 {
				t.Fatalf("expected one spawn-managed call, got %d (a claude PTY spawn means the setting was ignored)", len(managed))
			}
			if got := managed[0].body["provider"]; got != tc.want {
				t.Errorf("spawned on %v, want %q", got, tc.want)
			}
		})
	}
}

func TestExplicitProviderBeatsTheConfiguredRoleHarness(t *testing.T) {
	// The launcher offers a per-launch harness override; a config default must
	// not quietly reclaim it.
	reg, calls := roleProviderRig(t, map[string]any{
		"agents": map[string]any{"managerProvider": "codex"},
	})
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","manager":true,"provider":"claude","transport":"pty"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(calls("/sessions/spawn-managed")); n != 0 {
		t.Fatalf("explicit claude+pty went managed (%d calls) — the config default reclaimed the pick", n)
	}
	if n := len(calls("/sessions/spawn")); n != 1 {
		t.Fatalf("expected one PTY spawn, got %d", n)
	}
}

func TestUnknownConfiguredHarnessFallsBackToClaude(t *testing.T) {
	// A hand-edited config naming a harness we do not speak would otherwise
	// reach an adapter with no idea what it is; claude at least runs.
	reg, calls := roleProviderRig(t, map[string]any{
		"agents": map[string]any{"managerProvider": "gpt6"},
	})
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","manager":true,"transport":"pty"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(calls("/sessions/spawn")); n != 1 {
		t.Fatalf("expected the claude PTY fallback, got %d PTY spawns", n)
	}
}

func TestPlainWorkerIgnoresTheRoleHarnessSettings(t *testing.T) {
	// This setting is for the manager; it is not a global default provider
	// (that is agents.defaultProvider, applied by the spawn dialog).
	reg, calls := roleProviderRig(t, map[string]any{
		"agents": map[string]any{"managerProvider": "codex"},
	})
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","transport":"pty"}`)); err != nil {
		t.Fatal(err)
	}
	if n := len(calls("/sessions/spawn-managed")); n != 0 {
		t.Fatalf("a plain worker was routed to a managed provider (%d calls)", n)
	}
	if n := len(calls("/sessions/spawn")); n != 1 {
		t.Fatalf("expected one PTY spawn, got %d", n)
	}
}

func TestHeadlessManagerResolvesTheConfiguredSelectionTuple(t *testing.T) {
	reg, calls := roleProviderRig(t, map[string]any{
		"agents": map[string]any{
			"managerProvider": "codex",
			"managerModels": map[string]any{
				"claude": "opus",
				"codex":  "gpt-5-codex",
			},
			"managerEfforts": map[string]any{
				"claude": "high",
				"codex":  "xhigh",
			},
			"managerContextWindows": map[string]any{
				"claude": 200000,
				"codex":  400000,
			},
		},
	})
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","manager":true}`)); err != nil {
		t.Fatal(err)
	}
	managed := calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one managed spawn, got %d", len(managed))
	}
	want := map[string]any{
		"provider":       "codex",
		"model":          "gpt-5-codex",
		"model_identity": "gpt-5-codex",
		"effort":         "xhigh",
		"context_window": float64(400000),
	}
	for key, expected := range want {
		if got := managed[0].body[key]; got != expected {
			t.Errorf("%s = %#v, want %#v", key, got, expected)
		}
	}
}

func TestHeadlessManagerExplicitSelectionBeatsConfig(t *testing.T) {
	reg, calls := roleProviderRig(t, map[string]any{
		"agents": map[string]any{
			"managerProvider":       "codex",
			"managerModels":         map[string]any{"codex": "gpt-5-codex"},
			"managerEfforts":        map[string]any{"codex": "xhigh"},
			"managerContextWindows": map[string]any{"codex": 400000},
		},
	})
	if _, err := reg.handle(context.Background(), "agents.spawn", []byte(
		`{"cwd":"/tmp","manager":true,"model":"gpt-5.2","effort":"low","contextWindow":300000}`,
	)); err != nil {
		t.Fatal(err)
	}
	got := calls("/sessions/spawn-managed")[0].body
	if got["model"] != "gpt-5.2" || got["effort"] != "low" || got["context_window"] != float64(300000) {
		t.Fatalf("explicit tuple did not win: %#v", got)
	}
}

func TestHeadlessManagerExplicitProviderDefaultSuppressesFreshCodexOneMillion(t *testing.T) {
	reg, calls := roleProviderRig(t, map[string]any{
		"agents": map[string]any{
			"managerProvider":       "codex",
			"managerContextWindows": map[string]any{"codex": nil},
		},
	})
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","manager":true}`)); err != nil {
		t.Fatal(err)
	}
	if _, present := calls("/sessions/spawn-managed")[0].body["context_window"]; present {
		t.Fatalf("explicit provider-default was overwritten by the fresh Codex default: %#v",
			calls("/sessions/spawn-managed")[0].body)
	}
}

func TestHeadlessManagerResumeDoesNotTakeCurrentPreferences(t *testing.T) {
	reg, calls := roleProviderRig(t, map[string]any{
		"agents": map[string]any{
			"managerProvider":       "codex",
			"managerModels":         map[string]any{"codex": "gpt-5-codex"},
			"managerEfforts":        map[string]any{"codex": "xhigh"},
			"managerContextWindows": map[string]any{"codex": 400000},
		},
	})
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","manager":true,"resumeSessionId":"old-manager"}`)); err != nil {
		t.Fatal(err)
	}
	got := calls("/sessions/spawn-managed")[0].body
	for _, key := range []string{"model", "model_identity", "effort", "context_window"} {
		if value, present := got[key]; present {
			t.Errorf("resume unexpectedly inherited current %s=%#v", key, value)
		}
	}
	if got["resume"] != "old-manager" {
		t.Errorf("resume = %#v, want old-manager", got["resume"])
	}
}

func TestHeadlessClaudeManagerResumeDoesNotTakeCurrentOrGlobalDefaults(t *testing.T) {
	reg, calls := roleProviderRig(t, map[string]any{
		"claude": map[string]any{
			"transport":     "stream",
			"defaultModel":  "opus",
			"contextWindow": 1_000_000,
		},
		"agents": map[string]any{
			"managerProvider":       "claude",
			"managerModels":         map[string]any{"claude": "sonnet"},
			"managerEfforts":        map[string]any{"claude": "max"},
			"managerContextWindows": map[string]any{"claude": 200000},
		},
	})
	if _, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","manager":true,"resumeSessionId":"old-manager"}`)); err != nil {
		t.Fatal(err)
	}
	got := calls("/sessions/spawn-managed")[0].body
	for _, key := range []string{"model", "model_identity", "effort", "context_window"} {
		if value, present := got[key]; present {
			t.Errorf("Claude resume unexpectedly inherited %s=%#v", key, value)
		}
	}
	if got["resume"] != "old-manager" {
		t.Errorf("resume = %#v, want old-manager", got["resume"])
	}
}
