package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// ── firstUserMessage: the ORIGINAL dispatch, which retyping is what risks ────

func TestFirstUserMessagePicksTheOriginalDispatch(t *testing.T) {
	conv := `{"seq":9,"items":[
		{"kind":"assistant_text","text":"booting"},
		{"kind":"user_message","text":"SHIP TASK — fix the parser"},
		{"kind":"tool_use","name":"Edit"},
		{"kind":"user_message","text":"also do this other thing"}
	]}`
	if got := firstUserMessage(json.RawMessage(conv)); got != "SHIP TASK — fix the parser" {
		// The FIRST user turn is the dispatch; a later one is the scope creep
		// this tool exists to undo, so taking the last would clone the bug.
		t.Errorf("firstUserMessage = %q, want the first user turn", got)
	}
}

func TestFirstUserMessageEmptyWhenThereIsNoTask(t *testing.T) {
	for name, conv := range map[string]string{
		"no user turn": `{"seq":1,"items":[{"kind":"assistant_text","text":"hi"}]}`,
		"blank text":   `{"seq":1,"items":[{"kind":"user_message","text":"   "}]}`,
		"no items":     `{"seq":1,"items":[]}`,
		"not JSON":     `not json at all`,
	} {
		if got := firstUserMessage(json.RawMessage(conv)); got != "" {
			t.Errorf("%s: firstUserMessage = %q, want empty", name, got)
		}
	}
}

// ── the composed message ────────────────────────────────────────────────────

func TestRespawnHeadingIsUnmistakable(t *testing.T) {
	// The successor reads a message whose first half was written for a
	// DIFFERENT agent. If the correction reads as a footnote, the redispatch
	// repeats the original mistake.
	if !strings.Contains(respawnHeading, "CORRECTION") || !strings.Contains(respawnHeading, "supersedes") {
		t.Errorf("respawnHeading does not announce itself as superseding: %q", respawnHeading)
	}
}

func TestRetryLabelMarksTheSuccessorButInventsNothing(t *testing.T) {
	if got := retryLabel("alpha: parser fix"); got != "alpha: parser fix (redispatch)" {
		t.Errorf("retryLabel = %q", got)
	}
	// An unlabelled original stays unlabelled — naming an agent is the UI's job.
	if got := retryLabel("  "); got != "" {
		t.Errorf("retryLabel on a blank label = %q, want empty", got)
	}
}

func TestSessionIDFromSpawnResult(t *testing.T) {
	obj := &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: `{"sessionId":"abc-123"}`}}}
	if got := sessionIDFrom(obj); got != "abc-123" {
		t.Errorf("object form: got %q", got)
	}
	bare := &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: `"abc-123"`}}}
	if got := sessionIDFrom(bare); got != "abc-123" {
		t.Errorf("bare form: got %q", got)
	}
	// forward() normalizes an empty/null result to the literal "ok"; that is
	// not an id, and treating it as one would send the task into the void.
	for _, text := range []string{"ok", "", "{}", "not json"} {
		res := &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: text}}}
		if got := sessionIDFrom(res); got != "" {
			t.Errorf("%q should yield no id, got %q", text, got)
		}
	}
}

// ── end to end, against a fake hub ──────────────────────────────────────────

// respawnHub is a scripted bus: it answers the four methods respawn_with
// composes and records every call, so the test can assert on what was FORWARDED
// rather than on the tool's own summary.
type respawnHub struct {
	calls    []busCall
	snapshot string
	conv     string
	spawnErr bool
	// noMessageQueued makes agents.spawn answer WITHOUT `messageQueued` — a
	// provider that predates the first-message field (an older federated peer,
	// a lagging headless brain). The spawn looks perfectly successful and the
	// prompt went nowhere, which is the skew the fallback exists for.
	noMessageQueued bool
	sendErr         bool
}

type busCall struct {
	method string
	params map[string]any
}

func (h *respawnHub) Call(_ context.Context, method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var decoded map[string]any
	_ = json.Unmarshal(raw, &decoded)
	h.calls = append(h.calls, busCall{method: method, params: decoded})
	switch method {
	case "sessions.snapshot":
		return json.RawMessage(h.snapshot), nil
	case "sessions.conversation":
		return json.RawMessage(h.conv), nil
	case "agents.spawn":
		if h.spawnErr {
			return nil, errFake
		}
		msg, _ := decoded["message"].(string)
		if h.noMessageQueued || strings.TrimSpace(msg) == "" {
			return json.RawMessage(`{"sessionId":"new-1"}`), nil
		}
		// A current provider acknowledges that it took delivery of the prompt.
		return json.RawMessage(`{"sessionId":"new-1","messageQueued":true}`), nil
	case "agents.sendMessage":
		if h.sendErr {
			return nil, errFake
		}
		return json.RawMessage(`{"ok":true}`), nil
	case "config.get":
		return json.RawMessage(`{"claude":{}}`), nil
	}
	return json.RawMessage(`{}`), nil
}

func (h *respawnHub) call(method string) *busCall {
	for i := range h.calls {
		if h.calls[i].method == method {
			return &h.calls[i]
		}
	}
	return nil
}

type fakeErr struct{}

func (fakeErr) Error() string { return "spawn refused" }

var errFake = fakeErr{}

// callRespawn drives respawn_with through a real MCP client against a server
// whose bus calls are intercepted, and returns the tool result plus the hub log.
func callRespawn(t *testing.T, hub *respawnHub, yolo bool, args map[string]any) (*mcp.CallToolResult, *respawnHub) {
	t.Helper()
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	// newServerWithGrants takes a *busclient.Client, so the interception point
	// is the build's own caller: construct the server, then swap in the fake by
	// building the tool against it directly.
	srv := mcp.NewServer(&mcp.Implementation{Name: "workspacer-test", Version: "v1"}, nil)
	b := &build{
		s:     srv,
		c:     nil,
		scope: authtoken.ScopeOperator,
		allow: authtoken.ScopeOperator.Methods(),
		yolo:  yolo,
		caller: func(ctx context.Context, method string, params any) (json.RawMessage, error) {
			return hub.Call(ctx, method, params)
		},
	}
	addRespawnTool(b)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := srv.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	defer cs.Close()

	res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "respawn_with", Arguments: args})
	if err != nil {
		t.Fatalf("CallTool: %v", err)
	}
	return res, hub
}

func newRespawnHub() *respawnHub {
	return &respawnHub{
		snapshot: `{"cwd":"/w/alpha-wt","label":"alpha: parser","provider":"claude",
			"parentSessionId":"mgr-1","settings":{"model":"claude-opus-5","effort":"high",
			"permissionMode":"bypassPermissions"}}`,
		conv: `{"seq":4,"items":[{"kind":"user_message","text":"SHIP TASK — fix the parser"}]}`,
	}
}

func TestRespawnClonesTheTaskAndAppendsTheCorrection(t *testing.T) {
	res, hub := callRespawn(t, newRespawnHub(), true, map[string]any{
		"sessionId": "old-1",
		"amendment": "You rewrote the lexer. Do NOT touch it — only fix the off-by-one in parse().",
	})
	if res.IsError {
		t.Fatalf("unexpected error: %s", resultText(res))
	}

	spawn := hub.call("agents.spawn")
	if spawn == nil {
		t.Fatal("no agents.spawn call")
	}
	// Everything the manager would otherwise have had to remember.
	for field, want := range map[string]string{
		"cwd":             "/w/alpha-wt",
		"model":           "claude-opus-5",
		"effort":          "high",
		"provider":        "claude",
		"parentSessionId": "mgr-1",
		"label":           "alpha: parser (redispatch)",
	} {
		if got, _ := spawn.params[field].(string); got != want {
			t.Errorf("spawn %s = %q, want %q", field, got, want)
		}
	}

	// The composed dispatch rides the SPAWN ITSELF now — no round trip, and no
	// window in which the successor is live with no instructions.
	text, _ := spawn.params["message"].(string)
	if !strings.HasPrefix(text, "SHIP TASK — fix the parser") {
		t.Errorf("the ORIGINAL task must lead the message, got: %q", text)
	}
	if !strings.Contains(text, "off-by-one in parse()") {
		t.Errorf("the amendment is missing from the message: %q", text)
	}
	if !strings.Contains(text, "CORRECTION") {
		t.Errorf("the amendment is not announced as a correction: %q", text)
	}
	// And exactly once. Sending again after a spawn that already took the
	// prompt would make the successor read its whole dispatch twice.
	if send := hub.call("agents.sendMessage"); send != nil {
		t.Errorf("the task must not also be sent separately, got %v", send.params)
	}
}

// The skew case: a provider that does not know the field answers a normal spawn
// and drops the prompt. Without the fallback the successor sits idle with no
// task and nothing says so — indistinguishable from a wedge, and the exact
// failure class this field was added to remove.
func TestRespawnFallsBackToSendMessageWhenTheProviderDoesNotConfirm(t *testing.T) {
	hub := newRespawnHub()
	hub.noMessageQueued = true
	res, hub := callRespawn(t, hub, true, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	if res.IsError {
		t.Fatalf("unexpected error: %s", resultText(res))
	}
	send := hub.call("agents.sendMessage")
	if send == nil {
		t.Fatal("an unconfirmed spawn must fall back to sending the task")
	}
	if id, _ := send.params["sessionId"].(string); id != "new-1" {
		t.Errorf("task sent to %q, want the NEW session", id)
	}
	text, _ := send.params["text"].(string)
	if !strings.HasPrefix(text, "SHIP TASK — fix the parser") || !strings.Contains(text, "narrow it") {
		t.Errorf("the fallback must send the SAME composed dispatch, got: %q", text)
	}
}

// …and when the fallback itself fails, the tool says the agent is idle rather
// than reporting a dispatch that never arrived.
func TestRespawnReportsAnUndeliverableTask(t *testing.T) {
	hub := newRespawnHub()
	hub.noMessageQueued, hub.sendErr = true, true
	res, _ := callRespawn(t, hub, true, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	if !res.IsError {
		t.Fatalf("want an error result, got: %s", resultText(res))
	}
	if !strings.Contains(resultText(res), "idle") {
		t.Errorf("the failure must say the agent is idle and needs the task, got: %s", resultText(res))
	}
}

// The security property: respawn_with reads the ORIGINAL's permission mode, so
// it must be re-judged by the same grant check spawn_agent applies. A worker
// that ran bypassed does NOT make its clone bypassed for an ungranted caller.
func TestRespawnDoesNotInheritBypassWithoutTheGrant(t *testing.T) {
	_, granted := callRespawn(t, newRespawnHub(), true, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	if skip, _ := granted.call("agents.spawn").params["skipPermissions"].(bool); !skip {
		t.Error("a GRANTED caller's clone of a bypassed worker should keep the bypass")
	}

	_, ungranted := callRespawn(t, newRespawnHub(), false, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	skip, ok := ungranted.call("agents.spawn").params["skipPermissions"].(bool)
	if !ok {
		t.Fatal("skipPermissions must ride the wire EXPLICITLY, never be omitted")
	}
	if skip {
		t.Error("an UNGRANTED caller's clone must be clamped to approvals-on, even though the original ran bypassed")
	}
}

// The other direction of "clone the original", and the one the full-access
// grant put at risk: a worker that deliberately ran with approvals ON must not
// come back bypassed just because its dispatcher holds the grant. respawn_with
// forwards the original's mode EXPLICITLY in both directions, so it never falls
// into spawnWithGrants' omitted path where the grant would fill in a bypass.
func TestRespawnDoesNotUpgradeANonBypassedOriginalForAGrantedCaller(t *testing.T) {
	hub := newRespawnHub()
	hub.snapshot = `{"cwd":"/w/alpha","label":"alpha: parser","provider":"claude",
		"parentSessionId":"mgr-1","settings":{"model":"claude-opus-5","permissionMode":"default"}}`

	_, h := callRespawn(t, hub, true, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	skip, ok := h.call("agents.spawn").params["skipPermissions"].(bool)
	if !ok {
		t.Fatal("skipPermissions must ride the wire EXPLICITLY, never be omitted")
	}
	if skip {
		t.Error("a clone of a worker that ran with approvals ON must keep them on, grant or no grant")
	}
}

func TestRespawnRefusesWithoutAnAmendment(t *testing.T) {
	// A clone with no correction is a worker that will do the same thing again.
	res, hub := callRespawn(t, newRespawnHub(), true, map[string]any{
		"sessionId": "old-1", "amendment": "   ",
	})
	if !res.IsError || !strings.Contains(resultText(res), "amendment") {
		t.Errorf("want a refusal naming the amendment, got: %s", resultText(res))
	}
	if hub.call("agents.spawn") != nil {
		t.Error("nothing should have been spawned")
	}
}

func TestRespawnRefusesASessionWithNoTaskToClone(t *testing.T) {
	hub := newRespawnHub()
	hub.conv = `{"seq":0,"items":[]}`
	res, hub := callRespawn(t, hub, true, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	if !res.IsError || !strings.Contains(resultText(res), "no first user message") {
		t.Errorf("want a refusal, got: %s", resultText(res))
	}
	if hub.call("agents.spawn") != nil {
		// Launching with the amendment alone would be a dispatch the caller
		// never wrote.
		t.Error("nothing should have been spawned")
	}
}

func TestRespawnOverridesWinOverTheOriginal(t *testing.T) {
	_, hub := callRespawn(t, newRespawnHub(), true, map[string]any{
		"sessionId": "old-1",
		"amendment": "start clean",
		"model":     "claude-sonnet-5",
		"label":     "alpha: parser take 2",
		"cwd":       "/w/alpha",
		"worktree":  true,
		"toolScope": "view",
	})
	p := hub.call("agents.spawn").params
	for field, want := range map[string]any{
		"model":     "claude-sonnet-5",
		"label":     "alpha: parser take 2",
		"cwd":       "/w/alpha",
		"worktree":  true,
		"toolScope": "view",
	} {
		if p[field] != want {
			t.Errorf("spawn %s = %v, want %v", field, p[field], want)
		}
	}
}

// G5: the original was dispatched with a structured-result contract, so its
// snapshot records `resultSchema` (setSpawnMeta). Losing it on respawn
// silently downgrades a structured dispatch to a prose one — the caller gets
// prose back where it expected a validated object, with nothing announcing
// the downgrade.
func TestRespawnInheritsResultSchemaFromTheOriginalSnapshot(t *testing.T) {
	hub := newRespawnHub()
	hub.snapshot = `{"cwd":"/w/alpha-wt","label":"alpha: parser","provider":"claude",
		"parentSessionId":"mgr-1","settings":{"model":"claude-opus-5","effort":"high",
		"permissionMode":"bypassPermissions"},
		"resultSchema":{"type":"object","required":["commit"],"properties":{"commit":{"type":"string"}}}}`

	_, h := callRespawn(t, hub, true, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	spawn := h.call("agents.spawn")
	if spawn == nil {
		t.Fatal("no agents.spawn call")
	}
	schema, ok := spawn.params["resultSchema"].(map[string]any)
	if !ok {
		t.Fatalf("resultSchema was not forwarded to the spawn, got params: %#v", spawn.params)
	}
	if schema["type"] != "object" {
		t.Errorf("resultSchema.type = %v, want %q", schema["type"], "object")
	}
	required, _ := schema["required"].([]any)
	if len(required) != 1 || required[0] != "commit" {
		t.Errorf("resultSchema.required = %v, want [\"commit\"]", schema["required"])
	}
}

// An original spawned with no resultSchema must not fabricate one — a bare
// spawn should stay a bare spawn.
func TestRespawnOmitsResultSchemaWhenTheOriginalHadNone(t *testing.T) {
	_, hub := callRespawn(t, newRespawnHub(), true, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	if _, present := hub.call("agents.spawn").params["resultSchema"]; present {
		t.Error("resultSchema should be absent, not fabricated, when the original had none")
	}
}

func TestRespawnReportsAFailedSpawnInsteadOfSendingIntoTheVoid(t *testing.T) {
	hub := newRespawnHub()
	hub.spawnErr = true
	res, hub := callRespawn(t, hub, true, map[string]any{
		"sessionId": "old-1", "amendment": "narrow it",
	})
	if !res.IsError {
		t.Errorf("a failed spawn must surface as an error, got: %s", resultText(res))
	}
	if hub.call("agents.sendMessage") != nil {
		t.Error("no task should be sent when the spawn failed")
	}
}
