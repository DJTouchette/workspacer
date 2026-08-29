package main

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/djtouchette/workspacer-hub/internal/broker"
	"github.com/djtouchette/workspacer-hub/internal/bus"
	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// save_config is the TRIGGER half of the 2026-08-29 config-wipe defect.
//
// It was registered with addObjectTool, whose inferred schema is exactly
// {"type":"object","additionalProperties":true} — nothing below the top level is
// constrained. So a client that serialised its argument, sending
// {"projects": "{...}"} instead of {"projects": {...}}, was accepted here and
// forwarded verbatim to the bus, where the brain answered by coercing the
// non-map to {} — deleting every project the user had, and reporting success.
//
// The brain refusing (errWholesaleNotAMap) is the layer that has to hold for
// every OTHER caller: web, mobile, the TUI, plugins, a hand-written bus client.
// These tests are the door: the bad call should not arrive at all.

// recordingProvider is fakeProvider plus a log of the calls it actually
// received. "The facade returned an error" and "the facade wrote nothing" are
// different claims, and only the second one is the data-loss guarantee.
type recordingProvider struct {
	mu      sync.Mutex
	methods []string
}

func (r *recordingProvider) got() []string {
	r.mu.Lock()
	defer r.mu.Unlock()
	return append([]string(nil), r.methods...)
}

func startRecordingProvider(t *testing.T, ctx context.Context, busURL string, methods []string) *recordingProvider {
	t.Helper()
	rec := &recordingProvider{}
	conn, _, err := websocket.Dial(ctx, busURL, nil)
	if err != nil {
		t.Fatalf("provider dial: %v", err)
	}
	reg, _ := json.Marshal(busFrame{Op: "register", Methods: methods})
	if err := conn.Write(ctx, websocket.MessageText, reg); err != nil {
		t.Fatalf("provider register: %v", err)
	}
	go func() {
		defer conn.CloseNow()
		for {
			_, data, err := conn.Read(ctx)
			if err != nil {
				return
			}
			var f busFrame
			if json.Unmarshal(data, &f) != nil || f.Op != "call" {
				continue
			}
			rec.mu.Lock()
			rec.methods = append(rec.methods, f.Method)
			rec.mu.Unlock()
			out, _ := json.Marshal(map[string]any{
				"method": f.Method,
				"params": json.RawMessage(nonNil(f.Params)),
			})
			reply, _ := json.Marshal(busFrame{Op: "result", ID: f.ID, Result: out})
			_ = conn.Write(ctx, websocket.MessageText, reply)
		}
	}()
	return rec
}

// facadeWithRecorder wires a real hub + a recording provider + the facade + an
// in-memory MCP client, and hands back the client session and the recorder.
func facadeWithRecorder(t *testing.T, ctx context.Context) (*mcp.ClientSession, *recordingProvider) {
	t.Helper()
	hub := httptest.NewServer(bus.NewServer(broker.New()).Handler())
	t.Cleanup(hub.Close)
	busURL := strings.Replace(hub.URL, "http", "ws", 1) + "/bus"
	rec := startRecordingProvider(t, ctx, busURL, []string{"config.save"})

	client := busclient.New(busURL, "")
	go client.Run(ctx)
	server := newServer(client, authtoken.ScopeOperator)

	cT, sT := mcp.NewInMemoryTransports()
	if _, err := server.Connect(ctx, sT, nil); err != nil {
		t.Fatalf("server connect: %v", err)
	}
	mc := mcp.NewClient(&mcp.Implementation{Name: "test", Version: "v1"}, nil)
	cs, err := mc.Connect(ctx, cT, nil)
	if err != nil {
		t.Fatalf("client connect: %v", err)
	}
	t.Cleanup(func() { cs.Close() })
	return cs, rec
}

func callToolText(t *testing.T, res *mcp.CallToolResult) string {
	t.Helper()
	var sb strings.Builder
	for _, c := range res.Content {
		if tc, ok := c.(*mcp.TextContent); ok {
			sb.WriteString(tc.Text)
		}
	}
	return sb.String()
}

// TestSaveConfigDeclaresAShapeForTheWholesaleMaps pins the schema itself. An
// advertised schema is what the model reads; a bare open object told it nothing,
// and told the SDK nothing to validate.
func TestSaveConfigDeclaresAShapeForTheWholesaleMaps(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	cs, _ := facadeWithRecorder(t, ctx)

	tools, err := cs.ListTools(ctx, nil)
	if err != nil {
		t.Fatalf("ListTools: %v", err)
	}
	var raw []byte
	for _, tl := range tools.Tools {
		if tl.Name == "save_config" {
			raw, _ = json.Marshal(tl.InputSchema)
		}
	}
	if raw == nil {
		t.Fatal("save_config is not registered")
	}
	var schema map[string]any
	if err := json.Unmarshal(raw, &schema); err != nil {
		t.Fatalf("schema does not decode: %v", err)
	}
	// The document itself stays OPEN: a config patch is free-form and a new
	// config key must not need a facade release.
	if schema["additionalProperties"] != true {
		t.Errorf("save_config's schema is closed (additionalProperties=%v) — an ordinary new config key would be refused", schema["additionalProperties"])
	}
	if _, ok := schema["required"]; ok {
		t.Errorf("save_config's schema declares required keys; a partial patch names only what it changes")
	}
	// Every wholesale path must be typed as an object, at its own depth.
	for _, dotted := range wholesaleConfigPaths {
		node := schema
		for _, k := range strings.Split(dotted, ".") {
			props, ok := node["properties"].(map[string]any)
			if !ok {
				t.Fatalf("schema has no properties on the way to %s", dotted)
			}
			child, ok := props[k].(map[string]any)
			if !ok {
				t.Fatalf("schema does not describe %s (missing segment %q)", dotted, k)
			}
			node = child
		}
		if node["type"] != "object" {
			t.Errorf("schema types %s as %v, want \"object\" — the whole point is that a string here deletes the map", dotted, node["type"])
		}
	}
}

// TestSaveConfigSchemaMatchesTheContract holds the facade's copy of the
// wholesale list equal to contracts/wholesale-config-paths.json, so the door and
// the writer cannot come to disagree about which maps are destructive to get
// wrong.
func TestSaveConfigSchemaMatchesTheContract(t *testing.T) {
	raw, err := sweepguard.ReadRepoFile("contracts", "wholesale-config-paths.json")
	if err != nil {
		t.Fatalf("read contract fixture: %v", err)
	}
	var fixture struct {
		Paths []string `json:"paths"`
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatalf("parse contract fixture: %v", err)
	}
	if len(fixture.Paths) == 0 {
		t.Fatal("contract fixture has no paths — a silently empty fixture guards nothing")
	}
	want := map[string]bool{}
	for _, p := range fixture.Paths {
		want[p] = true
	}
	got := map[string]bool{}
	for _, p := range wholesaleConfigPaths {
		got[p] = true
	}
	for p := range want {
		if !got[p] {
			t.Errorf("contracts/wholesale-config-paths.json names %q and save_config's schema does not constrain it", p)
		}
	}
	for p := range got {
		if !want[p] {
			t.Errorf("save_config's schema constrains %q, which the contract does not list", p)
		}
	}
}

// TestSaveConfigRefusesAStringifiedWholesaleMap is the defect, at the door: the
// exact call shape that used to be forwarded and answered by emptying the map.
// The assertion that matters is the second one — the provider never saw a
// config.save at all, so there was nothing to write badly.
func TestSaveConfigRefusesAStringifiedWholesaleMap(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	for _, tc := range []struct {
		name string
		args map[string]any
	}{
		{"projects as a JSON string", map[string]any{"projects": `{"/w/a":{"label":"A"}}`}},
		{"projects as null", map[string]any{"projects": nil}},
		{"projects as an array", map[string]any{"projects": []any{}}},
		{"ui.customThemes as a string", map[string]any{"ui": map[string]any{"customThemes": "nord"}}},
		{"claude.budgets as a number", map[string]any{"claude": map[string]any{"budgets": 0}}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cs, rec := facadeWithRecorder(t, ctx)
			res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "save_config", Arguments: tc.args})
			if err != nil {
				// A protocol error is also a refusal, and also writes nothing.
				if len(rec.got()) != 0 {
					t.Fatalf("refused at the protocol level but still called %v", rec.got())
				}
				return
			}
			if !res.IsError {
				t.Fatalf("save_config ACCEPTED %v and answered %q", tc.args, callToolText(t, res))
			}
			if len(rec.got()) != 0 {
				t.Errorf("save_config reported an error but still forwarded %v to the bus — the write is what deletes the map", rec.got())
			}
		})
	}
}

// TestSaveConfigStillAcceptsRealPatches is the other direction: the guard must
// not have closed the door on the ordinary calls. A schema that refuses valid
// config writes is a worse bug than the one it replaced.
func TestSaveConfigStillAcceptsRealPatches(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	for _, tc := range []struct {
		name string
		args map[string]any
	}{
		{"an ordinary scalar patch", map[string]any{"ui": map[string]any{"guiFontScale": 1.3}}},
		{"a full projects map", map[string]any{"projects": map[string]any{
			"/w/a": map[string]any{"label": "A", "yolo": true},
		}}},
		{"an empty projects map (the documented way to empty it)", map[string]any{"projects": map[string]any{}}},
		{"a nested wholesale map", map[string]any{"ui": map[string]any{
			"theme": "nord", "customThemes": map[string]any{"nord": map[string]any{"bg": "#2e3440"}},
		}}},
		{"a config key the schema has never heard of", map[string]any{"pluginSettings": map[string]any{"x": 1}}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			cs, rec := facadeWithRecorder(t, ctx)
			res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "save_config", Arguments: tc.args})
			if err != nil {
				t.Fatalf("save_config refused a legal patch: %v", err)
			}
			if res.IsError {
				t.Fatalf("save_config refused a legal patch: %s", callToolText(t, res))
			}
			if got := rec.got(); len(got) != 1 || got[0] != "config.save" {
				t.Errorf("provider saw %v, want one config.save", got)
			}
		})
	}
}

// TestInvalidWholesaleValueIsALoadBearingSecondCheck exercises the facade's own
// guard directly.
//
// It has to be tested directly BECAUSE the declared schema rejects these calls
// first, so no end-to-end test can reach it. That is the point of it existing —
// "the SDK validates the schema" is a claim about a dependency, and the thing
// behind it is silent deletion of a user's project list — but a guard nothing
// exercises is a guard that has never been seen to work.
func TestInvalidWholesaleValueIsALoadBearingSecondCheck(t *testing.T) {
	for _, tc := range []struct {
		name     string
		args     map[string]any
		wantPath string
	}{
		{"a stringified projects map", map[string]any{"projects": "{}"}, "projects"},
		{"a null projects map", map[string]any{"projects": nil}, "projects"},
		{"an array", map[string]any{"projects": []any{}}, "projects"},
		{"a number at a nested path", map[string]any{"claude": map[string]any{"budgets": float64(0)}}, "claude.budgets"},
		{"a string at a nested path", map[string]any{"ui": map[string]any{"customThemes": "nord"}}, "ui.customThemes"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			path, _, bad := invalidWholesaleValue(tc.args)
			if !bad {
				t.Fatalf("invalidWholesaleValue accepted %v", tc.args)
			}
			if path != tc.wantPath {
				t.Errorf("blamed %q, want %q", path, tc.wantPath)
			}
		})
	}

	for _, tc := range []struct {
		name string
		args map[string]any
	}{
		{"an absent wholesale path", map[string]any{"ui": map[string]any{"theme": "nord"}}},
		{"a present, empty wholesale map", map[string]any{"projects": map[string]any{}}},
		{"a populated wholesale map", map[string]any{"projects": map[string]any{"/w/a": map[string]any{}}}},
		{"a same-named key at another depth", map[string]any{"supervisor": map[string]any{"budgets": "whatever"}}},
		{"a parent that is not an object at all", map[string]any{"ui": "nord"}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if path, _, bad := invalidWholesaleValue(tc.args); bad {
				t.Errorf("invalidWholesaleValue refused a legal patch, blaming %q", path)
			}
		})
	}
}
