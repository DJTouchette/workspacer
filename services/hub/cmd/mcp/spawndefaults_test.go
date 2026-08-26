package main

import (
	"bytes"
	"context"
	"log"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Omitted-skipPermissions default resolution, facade half. The desktop spawn
// dialog pre-selects the config default (claude.skipPermissionsDefault / a
// bypass defaultPermissionMode); a spawn_agent call that omits the field must
// resolve the SAME default — through the SAME grant gate as an explicit
// request, so the operator's default never escalates an ungranted session
// token. These run the real chain like yologrant_test.go: MCP client →
// spawn_agent handler → busclient → a REAL hub bus → an echoing provider that
// also answers config.get.

// spawnDefaultsSession is yoloGrantSession with a config.get answer beside the
// agents.spawn echo, so the facade's default resolution has a config to read.
func spawnDefaultsSession(t *testing.T, ctx context.Context, yolo bool, claudeCfg map[string]any) *mcp.ClientSession {
	t.Helper()
	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	answeringProvider(t, ctx, busURL, map[string]any{
		"agents.spawn": nil, // nil = echo method+params
		"config.get":   map[string]any{"claude": claudeCfg},
	})

	client := busclient.New(busURL, "")
	go client.Run(ctx)

	server := newServerWithGrants(client, authtoken.ScopeOperator, nil, nil, yolo)
	return connectTo(t, ctx, server)
}

// TestSpawnAgentOmittedSkipResolvesConfigDefaultForAGrantedSession: granted
// token + omitted field + claude.skipPermissionsDefault:true → the worker
// spawns bypassed, exactly like the desktop dialog's pre-selected toggle.
func TestSpawnAgentOmittedSkipResolvesConfigDefaultForAGrantedSession(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{"skipPermissionsDefault": true})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if params["skipPermissions"] != true {
		t.Fatalf("granted session + config default on: omitted skipPermissions must resolve to true, got %v", params)
	}
	if params["yoloGranted"] != true {
		t.Fatalf("hub did not stamp yoloGranted on the defaulted spawn: %v", params)
	}
}

// TestSpawnAgentOmittedSkipHonorsABypassDefaultPermissionMode: the other config
// spelling of the same default — defaultPermissionMode:"bypassPermissions" with
// the toggle off.
func TestSpawnAgentOmittedSkipHonorsABypassDefaultPermissionMode(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{
		"skipPermissionsDefault": false,
		"defaultPermissionMode":  "bypassPermissions",
	})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if params["skipPermissions"] != true {
		t.Fatalf("a bypass defaultPermissionMode must resolve the omitted field to true, got %v", params)
	}
}

// TestSpawnAgentConfigDefaultIsClampedAndLoggedForAnUngrantedSession: the
// SECURITY half. The config default passes the SAME gate as an explicit
// request — without the full-access grant it is clamped, the strip is logged
// with its config-default provenance, and the wire carries an EXPLICIT false
// (never nil): the hub stamps yoloGranted on the facade's trusted host-token
// connection regardless of session, so a nil left on the wire would let the
// provider's own default resolution escalate the ungranted token.
func TestSpawnAgentConfigDefaultIsClampedAndLoggedForAnUngrantedSession(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	var buf bytes.Buffer
	prev := log.Writer()
	log.SetOutput(&buf)
	defer log.SetOutput(prev)

	cs := spawnDefaultsSession(t, ctx, false, map[string]any{"skipPermissionsDefault": true})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "label": "worker-d"})

	v, present := params["skipPermissions"]
	if !present || v != false {
		t.Fatalf("ungranted session must forward an EXPLICIT skipPermissions:false (got present=%v value=%v) — an omitted field re-resolves the config default provider-side under the facade's yoloGranted stamp", present, v)
	}
	out := buf.String()
	if !strings.Contains(out, "config-defaulted") || !strings.Contains(out, "full-access grant") ||
		!strings.Contains(out, `"worker-d"`) {
		t.Fatalf("clamped config default must be logged with its provenance + agent label, got:\n%s", out)
	}
}

// TestSpawnAgentOmittedSkipWithDefaultOffStaysOffWithoutTheGrant: default off +
// omitted field + no grant → approvals on, and still an explicit false on the
// wire.
func TestSpawnAgentOmittedSkipWithDefaultOffStaysOffWithoutTheGrant(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, false, map[string]any{"skipPermissionsDefault": false})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if v, present := params["skipPermissions"]; !present || v != false {
		t.Fatalf("default off, no grant: omitted skipPermissions must forward explicit false, got present=%v value=%v", present, v)
	}
}

// TestSpawnAgentGrantAddsTheBypassToAnOmittedField: THE gap-#2 behaviour. The
// full-access grant exists only because config says the manager/supervisor's
// dispatched agents skip approvals (agents.fleetFullAccess / a per-project yolo
// / supervisor.fullAccess), so a granted session that OMITS skipPermissions
// gets the bypass — the grant ADDS it, it does not merely honor a caller that
// happened to pass it. Before this, a manager dispatching without the magic
// word watched every worker prompt on every Bash call with full access visibly
// ON, and there was no way to tell from the outside why.
//
// Note the config here has the default OFF: the grant alone is enough.
func TestSpawnAgentGrantAddsTheBypassToAnOmittedField(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{"skipPermissionsDefault": false})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if params["skipPermissions"] != true {
		t.Fatalf("granted session + omitted skipPermissions must resolve to true (the grant IS the operator's intent), got %v", params)
	}
	if params["yoloGranted"] != true {
		t.Fatalf("hub did not stamp yoloGranted on the granted spawn: %v", params)
	}
}

// TestSpawnAgentGrantDoesNotOverrideAnExplicitFalse: the grant fills in an
// OMITTED field only. A manager that deliberately dispatches one worker with
// approvals on — a worker about to touch something it should be gated on —
// still gets what it asked for.
func TestSpawnAgentGrantDoesNotOverrideAnExplicitFalse(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{"skipPermissionsDefault": false})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "skipPermissions": false})
	if v, present := params["skipPermissions"]; !present || v != false {
		t.Fatalf("explicit false must beat the grant, got present=%v value=%v", present, v)
	}
}

// TestSpawnAgentExplicitFalseBeatsTheConfigDefault: an explicit caller value —
// including false — always wins over the config default.
func TestSpawnAgentExplicitFalseBeatsTheConfigDefault(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, true, map[string]any{"skipPermissionsDefault": true})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "skipPermissions": false})
	if v, present := params["skipPermissions"]; !present || v != false {
		t.Fatalf("explicit false must beat the config default, got present=%v value=%v", present, v)
	}
}

// Omitted-model default resolution. A dispatch that names no model must
// resolve to the workspacer config default (claude.defaultModel) — the same
// value the desktop spawn dialog pre-fills — so a worker a Fleet Manager
// dispatches plainly inherits the SAME model (including a configured `[1m]`
// 1M-context variant) the manager itself is likely running on. Before this,
// an omitted model reached the provider as "", `claude` picked its own
// default with no `--model` flag at all, and a fleet's workers silently ran
// on a smaller context window than their manager.

// TestSpawnAgentOmittedModelResolvesConfigDefault: the plain case — no model
// named, config has one — the omitted field must resolve to it.
func TestSpawnAgentOmittedModelResolvesConfigDefault(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, false, map[string]any{"defaultModel": "opus[1m]"})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if params["model"] != "opus[1m]" {
		t.Fatalf("omitted model must resolve to the config default %q, got %v", "opus[1m]", params)
	}
}

// TestSpawnAgentExplicitModelBeatsTheConfigDefault: a caller that names a
// model — including one without a `[1m]` marker, e.g. for a deliberately
// cheaper/smaller worker — always wins over the config default. The default
// resolution must never overwrite an explicit request.
func TestSpawnAgentExplicitModelBeatsTheConfigDefault(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, false, map[string]any{"defaultModel": "opus[1m]"})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "model": "claude-haiku-4-5"})
	if params["model"] != "claude-haiku-4-5" {
		t.Fatalf("explicit model must beat the config default, got %v", params)
	}
}

// TestSpawnAgentOmittedModelWithNoConfigDefaultStaysEmpty: no config default
// set — the omitted field must forward as absent, not manufacture a model id.
func TestSpawnAgentOmittedModelWithNoConfigDefaultStaysEmpty(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, false, map[string]any{})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp"})
	if v, present := params["model"]; present && v != "" {
		t.Fatalf("no config default: omitted model must stay empty, got present=%v value=%v", present, v)
	}
}

// The config default is CLAUDE'S, and it must not ride a managed provider's
// spawn. `claude.defaultModel` holds a Claude model id ("opus[1m]"); codex,
// opencode and pi have their own model vocabularies. Handing one of them this
// value does not fail at spawn — the session opens, the dispatch is delivered
// verbatim, the provider starts a turn, and the API rejects THE TURN ("The
// 'opus[1m]' model is not supported when using Codex with a ChatGPT account").
// What the operator sees is an agent that opened, answered nothing and ended,
// which reads as "the initial message never reached it" while the message was
// never the problem. The desktop spawn dialog already clears the model when a
// managed provider is picked; this pins the facade to the same rule.
func TestSpawnAgentConfigDefaultModelIsNotAppliedToManagedProviders(t *testing.T) {
	for _, provider := range []string{"codex", "opencode", "pi"} {
		t.Run(provider, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
			defer cancel()

			cs := spawnDefaultsSession(t, ctx, false, map[string]any{"defaultModel": "opus[1m]"})
			params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "provider": provider})
			if v, present := params["model"]; present && v != "" {
				t.Fatalf("%s spawn must not inherit claude.defaultModel (got %v) — the provider's own default applies when no model is named", provider, v)
			}
		})
	}
}

// The claude arm of the same rule, spelled with an EXPLICIT provider rather
// than the omitted one TestSpawnAgentOmittedModelResolvesConfigDefault covers,
// so a fix that gated on "provider was omitted" instead of "provider is
// claude" fails here.
func TestSpawnAgentConfigDefaultModelStillAppliesToAnExplicitClaudeProvider(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, false, map[string]any{"defaultModel": "opus[1m]"})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "provider": "claude"})
	if params["model"] != "opus[1m]" {
		t.Fatalf("an explicit claude provider must still resolve the config default, got %v", params)
	}
}

// An explicitly named model always wins, for a managed provider too — the gate
// above must not become "managed providers never get a model".
func TestSpawnAgentExplicitModelSurvivesForAManagedProvider(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Second)
	defer cancel()

	cs := spawnDefaultsSession(t, ctx, false, map[string]any{"defaultModel": "opus[1m]"})
	params := spawnEchoParams(t, ctx, cs, map[string]any{"cwd": "/tmp", "provider": "codex", "model": "gpt-5.1-codex"})
	if params["model"] != "gpt-5.1-codex" {
		t.Fatalf("explicit model must survive for a managed provider, got %v", params)
	}
}
