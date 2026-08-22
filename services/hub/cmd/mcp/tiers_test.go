package main

import (
	"context"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// listToolsFor connects an in-memory MCP client to one tier's server and
// returns its tool names.
func listToolsFor(t *testing.T, scope authtoken.Scope) map[string]bool {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	client := busclient.New("ws://127.0.0.1:0/bus", "")
	server := newServer(client, scope)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	defer cs.Close()

	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("ListTools: %v", err)
	}
	out := map[string]bool{}
	for _, tl := range tools.Tools {
		out[tl.Name] = true
	}
	return out
}

// TestTierToolFiltering pins the tier → tool derivation: a tier holds exactly
// the tools whose hub method its authtoken scope may call. If a method is
// admitted to viewMethods/triageMethods later, its tool lights up here with no
// facade change — and a tool leaking into a lower tier is a test failure.
func TestTierToolFiltering(t *testing.T) {
	view := listToolsFor(t, authtoken.ScopeView)
	triage := listToolsFor(t, authtoken.ScopeTriage)
	operator := listToolsFor(t, authtoken.ScopeOperator)

	// View: observation only.
	for _, want := range []string{"list_agents", "get_transcript", "get_conversation", "get_snapshot", "list_snapshots", "get_host_cwd", "get_config", "help"} {
		if !view[want] {
			t.Errorf("view tier missing %q", want)
		}
	}
	for _, banned := range []string{"spawn_agent", "send_message", "approve", "write_file", "read_file", "save_config", "create_terminal", "terminal_input", "signal", "notify", "list_jobs", "propose_job", "run_job", "remove_job", "job_history",
		// The Fleet Manager tools. A view SCOUT is dispatched read-only and
		// must not be able to append to the user's briefs, dismiss sessions,
		// redispatch workers, or arm wakes at other sessions.
		"brief_append", "close_session", "respawn_with", "notify_when", "project_status"} {
		if view[banned] {
			t.Errorf("view tier must not hold %q", banned)
		}
	}

	// Triage: view + acting on attention, still no spawn / fs / config writes.
	for _, want := range []string{"list_agents", "approve", "send_message", "signal", "help"} {
		if !triage[want] {
			t.Errorf("triage tier missing %q", want)
		}
	}
	for _, banned := range []string{"spawn_agent", "create_terminal", "write_file", "read_file", "save_config", "terminal_input", "answer", "list_jobs", "propose_job", "run_job", "remove_job", "job_history",
		// Same for triage: it may act on ATTENTION (approve, reply, interrupt)
		// and nothing else. respawn_with in particular composes agents.spawn,
		// so admitting it here would hand triage the spawn it is defined not to
		// have — its own gate derives from the parts for exactly that reason.
		"brief_append", "close_session", "respawn_with", "notify_when", "project_status"} {
		if triage[banned] {
			t.Errorf("triage tier must not hold %q", banned)
		}
	}

	// Operator: everything, help included.
	for _, want := range []string{"spawn_agent", "write_file", "save_config", "terminal_input", "answer", "help",
		"list_jobs", "job_history", "propose_job", "run_job", "remove_job",
		"brief_append", "close_session", "respawn_with", "notify_when", "project_status"} {
		if !operator[want] {
			t.Errorf("operator tier missing %q", want)
		}
	}

	// The job surface an agent gets is deliberately ASYMMETRIC: it may propose
	// a job, never upsert one, because a proposal lands disarmed and a
	// jobs.upsert would not. An "upsert_job"/"save_job" tool appearing at any
	// tier means that guarantee was traded away — the review step in
	// Settings → Jobs would then be decoration.
	for tier, tools := range map[string]map[string]bool{"view": view, "triage": triage, "operator": operator} {
		for _, banned := range []string{"upsert_job", "save_job", "create_job", "edit_job", "enable_job"} {
			if tools[banned] {
				t.Errorf("%s tier holds %q — agents must only ever PROPOSE jobs", tier, banned)
			}
		}
	}

	// The tiers are strictly nested and meaningfully different in size — the
	// whole point is that a view worker pays for a fraction of the schemas.
	if len(view) >= len(triage) || len(triage) >= len(operator) {
		t.Errorf("tier sizes not strictly increasing: view=%d triage=%d operator=%d", len(view), len(triage), len(operator))
	}
	for name := range view {
		if !triage[name] {
			t.Errorf("view tool %q missing from triage (tiers must nest)", name)
		}
	}
	for name := range triage {
		if !operator[name] {
			t.Errorf("triage tool %q missing from operator (tiers must nest)", name)
		}
	}
}

// TestHelpRendersFromRegistry proves help's docs come from the tier's actual
// registry: the overview lists only tools the tier holds, and a topic expands
// with guidance.
func TestHelpRendersFromRegistry(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	client := busclient.New("ws://127.0.0.1:0/bus", "")
	server := newServer(client, authtoken.ScopeView)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	defer cs.Close()

	overview, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "help", Arguments: map[string]any{}})
	if err != nil {
		t.Fatalf("help: %v", err)
	}
	text := textOf(overview)
	if !strings.Contains(text, "tier: view") || !strings.Contains(text, "list_agents") {
		t.Errorf("overview missing tier/tools: %s", text)
	}
	if strings.Contains(text, "spawn_agent") {
		t.Errorf("view help must not mention operator tools: %s", text)
	}

	detail, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "help", Arguments: map[string]any{"topic": "observe"}})
	if err != nil {
		t.Fatalf("help observe: %v", err)
	}
	dt := textOf(detail)
	if !strings.Contains(dt, "sinceSeq") || !strings.Contains(dt, "get_conversation") {
		t.Errorf("observe guidance missing: %s", dt)
	}

	unknown, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "help", Arguments: map[string]any{"topic": "nope"}})
	if err != nil {
		t.Fatalf("help nope: %v", err)
	}
	if !strings.Contains(textOf(unknown), "unknown topic") {
		t.Errorf("expected unknown-topic message, got: %s", textOf(unknown))
	}
}

// TestUiToolTierGate pins the explicit tier decision for the event-backed UI
// navigation tools: absent from view (a read-only worker must not steer the
// user's screen), present from triage up. Event-backed tools can't ride the
// derived method allowlists, so this pin is the only thing holding the line.
func TestUiToolTierGate(t *testing.T) {
	view := listToolsFor(t, authtoken.ScopeView)
	triage := listToolsFor(t, authtoken.ScopeTriage)
	operator := listToolsFor(t, authtoken.ScopeOperator)

	uiTools := []string{"focus_agent", "open_pane", "open_browser", "open_plugin", "open_spawn_dialog", "open_guide"}
	for _, name := range uiTools {
		if view[name] {
			t.Errorf("view tier must not hold UI tool %q", name)
		}
		if !triage[name] {
			t.Errorf("triage tier missing UI tool %q", name)
		}
		if !operator[name] {
			t.Errorf("operator tier missing UI tool %q", name)
		}
	}
}
