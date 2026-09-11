package main

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type workflowDispatchHub struct {
	calls   []busCall
	prepare string
	route   string
	spawn   string
	fail    string
}

func (h *workflowDispatchHub) call(_ context.Context, method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var p map[string]any
	_ = json.Unmarshal(raw, &p)
	h.calls = append(h.calls, busCall{method: method, params: p})
	if method == h.fail {
		return nil, errors.New("connection lost")
	}
	switch method {
	case "fleetWorkflows.request":
		if h.prepare != "" {
			return json.RawMessage(h.prepare), nil
		}
		return json.RawMessage(`{"ok":true,"dispatch":{"taskId":"task","cwd":"/repo","stepId":"review","expectedTaskRevision":4,"role":"reviewer","stage":"review","template":"review-task","toolScope":"view","afterDispatchId":"prior","previousProvider":"claude"}}`), nil
	case "routing.select", "fleet.selectDispatchModel":
		if h.route != "" {
			return json.RawMessage(h.route), nil
		}
		return json.RawMessage(`{"eligible":true,"role":"reviewer","provider":"codex","model":"gpt-5.4","effort":"high","capability":"reviewer","decisionId":"decision","independentFamily":true,"reason":["capacity available"]}`), nil
	case "agents.spawn":
		if h.spawn != "" {
			return json.RawMessage(h.spawn), nil
		}
		return json.RawMessage(`{"sessionId":"worker","taskId":"task","dispatchId":"dispatch","messageQueued":true,"renderedMessage":"large repeated prompt","escalationScrubbed":["toolScope"],"note":"host warning"}`), nil
	case "config.get":
		return json.RawMessage(`{"claude":{}}`), nil
	case "agents.notifyWhen":
		return json.RawMessage(`{"armed":true}`), nil
	}
	return nil, errors.New("unexpected call: " + method)
}
func dispatchArgs() map[string]any {
	return map[string]any{"taskId": "task", "cwd": "/repo", "stepId": "review", "expectedTaskRevision": 4, "templateParams": map[string]string{"task": "review acceptance criteria", "handoff": "facts only"}}
}
func workflowClient(t *testing.T, h *workflowDispatchHub, yolo bool) (context.Context, *mcp.ClientSession) {
	t.Helper()
	ctx := context.WithValue(context.Background(), tokenLabelKey{}, "session:manager")
	cs := connectToolClient(t, ctx, func(b *build) { b.caller = h.call; b.yolo = yolo; addWorkflowDispatchTool(b) })
	return ctx, cs
}
func TestComposedWorkflowDispatchRoutesAndSpawnsWithExistingGrants(t *testing.T) {
	for _, yolo := range []bool{false, true} {
		t.Run(map[bool]string{false: "ungranted", true: "granted"}[yolo], func(t *testing.T) {
			h := &workflowDispatchHub{}
			ctx, cs := workflowClient(t, h, yolo)
			args := dispatchArgs()
			args["skipPermissions"] = true
			args["watchContextUsedPct"] = 80
			result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "dispatch_workflow_step", Arguments: args})
			if err != nil || result.IsError {
				t.Fatalf("%v %v", result, err)
			}
			var methods []string
			for _, c := range h.calls {
				methods = append(methods, c.method)
			}
			if strings.Join(methods, ",") != "fleetWorkflows.request,routing.select,agents.spawn,agents.notifyWhen" {
				t.Fatal(methods)
			}
			route := h.calls[1].params
			spawn := h.calls[2].params
			if route["previousProvider"] != "claude" || route["role"] != "reviewer" || route["cwd"] != "/repo" {
				t.Fatal(route)
			}
			for key, want := range map[string]any{"provider": "codex", "modelIdentity": "gpt-5.4", "effort": "high", "role": "reviewer", "capability": "reviewer", "decisionId": "decision", "dispatchOwnerSessionId": "manager", "parentSessionId": "manager", "afterDispatchId": "prior", "workflowStepId": "review", "expectedTaskRevision": float64(4), "skipPermissions": yolo} {
				if spawn[key] != want {
					t.Fatalf("%s: got %v want %v", key, spawn[key], want)
				}
			}
			body := resultText(result)
			for _, kept := range []string{"worker", "dispatch", "escalationScrubbed", "host warning", "selection", "armed"} {
				if !strings.Contains(body, kept) {
					t.Fatalf("lost receipt %s: %s", kept, body)
				}
			}
			if strings.Contains(body, "large repeated prompt") {
				t.Fatal("prompt echoed")
			}
		})
	}
}
func TestComposedWorkflowDispatchStopsAtEveryRefusal(t *testing.T) {
	cases := []struct {
		name  string
		hub   workflowDispatchHub
		phase string
		calls int
	}{
		{"ownership", workflowDispatchHub{prepare: `{"ok":false,"error":"foreign task"}`}, "foreign task", 1},
		{"conditional skipped", workflowDispatchHub{prepare: `{"ok":true,"skipped":true,"taskRevision":5,"instructions":"next step"}`}, "skipped", 1},
		{"ineligible route", workflowDispatchHub{route: `{"eligible":false,"reason":["capacity exhausted"]}`}, "capacity exhausted", 2},
		{"missing eligibility", workflowDispatchHub{route: `{"error":"unavailable"}`}, "unavailable", 2},
		{"missing decision", workflowDispatchHub{route: `{"eligible":true,"provider":"codex","model":"gpt-5.4","role":"reviewer","capability":"reviewer"}`}, "incomplete", 2},
		{"lost spawn ack", workflowDispatchHub{fail: "agents.spawn"}, "admissionUncertain", 4},
		{"unaddressable spawn", workflowDispatchHub{spawn: `{"messageQueued":true}`}, "admissionUncertain", 4},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			ctx, cs := workflowClient(t, &tc.hub, false)
			result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "dispatch_workflow_step", Arguments: dispatchArgs()})
			if err != nil || !strings.Contains(resultText(result), tc.phase) {
				t.Fatalf("%v %v", result, err)
			}
			if len(tc.hub.calls) != tc.calls {
				t.Fatalf("unexpected calls: %#v", tc.hub.calls)
			}
		})
	}
}
func TestComposedWorkflowDispatchPairedAndWatchFailure(t *testing.T) {
	h := &workflowDispatchHub{}
	ctx, cs := workflowClient(t, h, false)
	args := dispatchArgs()
	args["executionTarget"] = "paired"
	args["remoteCwd"] = "/remote/repo"
	result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "dispatch_workflow_step", Arguments: args})
	if err != nil || result.IsError {
		t.Fatalf("%v %v", result, err)
	}
	if h.calls[1].method != "fleet.selectDispatchModel" || h.calls[1].params["cwd"] != "/remote/repo" {
		t.Fatal(h.calls)
	}
	spawn := h.calls[len(h.calls)-1]
	if spawn.params["cwd"] != "/repo" || spawn.params["remoteCwd"] != "/remote/repo" || spawn.params["executionTarget"] != "paired" {
		t.Fatal(spawn)
	}
	h = &workflowDispatchHub{fail: "agents.notifyWhen"}
	ctx, cs = workflowClient(t, h, false)
	args = dispatchArgs()
	args["watchContextUsedPct"] = 80
	result, err = cs.CallTool(ctx, &mcp.CallToolParams{Name: "dispatch_workflow_step", Arguments: args})
	if err != nil || result.IsError || !strings.Contains(resultText(result), "watchError") {
		t.Fatalf("%v %v", result, err)
	}
	n := 0
	for _, c := range h.calls {
		if c.method == "agents.spawn" {
			n++
		}
	}
	if n != 1 {
		t.Fatalf("spawn count %d", n)
	}
}
func TestComposedWorkflowPublicInputsAndTier(t *testing.T) {
	for _, name := range []string{"dispatch_workflow_step", "manager_context"} {
		if !listToolsFor(t, authtoken.ScopeOperator)[name] || listToolsFor(t, authtoken.ScopeView)[name] || listToolsFor(t, authtoken.ScopeTriage)[name] {
			t.Fatal(name)
		}
	}
	for _, key := range []string{"callerSessionId", "parentSessionId", "role", "model", "capability", "template", "hub", "resumeSessionId"} {
		t.Run(key, func(t *testing.T) {
			h := &workflowDispatchHub{}
			ctx, cs := workflowClient(t, h, false)
			args := dispatchArgs()
			args[key] = "forged"
			result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "dispatch_workflow_step", Arguments: args})
			if err != nil || !result.IsError || len(h.calls) > 0 {
				t.Fatalf("forged field forwarded: %v %v", result, err)
			}
		})
	}
}
func TestManagerContextComposesIndependentReadsWithBoundedEvidence(t *testing.T) {
	ctx := context.WithValue(context.Background(), tokenLabelKey{}, "session:manager")
	var mu sync.Mutex
	calls := 0
	gate := make(chan struct{})
	cs := connectToolClient(t, ctx, func(b *build) {
		b.caller = func(_ context.Context, _ string, params any) (json.RawMessage, error) {
			wire := params.(map[string]any)
			if wire["callerSessionId"] != "manager" {
				t.Error(wire)
			}
			mu.Lock()
			calls++
			if calls == 3 {
				close(gate)
			}
			mu.Unlock()
			<-gate
			if wire["op"] == "requestInbox" {
				if wire["view"] != "pending" {
					t.Error(wire)
				}
				return json.RawMessage(`{"ok":true,"requests":[],"remaining":0}`), nil
			}
			if wire["taskId"] == "foreign" {
				return nil, errors.New("Task unavailable")
			}
			return json.RawMessage(`{"ok":true,"task":{"taskId":"task","revision":8,"workflow":{"templates":{"x":{"body":"NEVER RETURN"}},"steps":[{"outcome":{"n":9007199254740993,"verdict":"failed"}}]},"attempts":["OLD HISTORY"]},"instructions":"inspect outcome"}`), nil
		}
		addManagerContextTool(b)
	})
	result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "manager_context", Arguments: map[string]any{"tasks": []map[string]string{{"taskId": "task", "cwd": "/repo"}, {"taskId": "foreign", "cwd": "/repo"}}}})
	if err != nil || result.IsError {
		t.Fatalf("%v %v", result, err)
	}
	body := resultText(result)
	for _, term := range []string{"9007199254740993", "failed", "Task unavailable", "inspect outcome"} {
		if !strings.Contains(body, term) {
			t.Fatal(body)
		}
	}
	if strings.Contains(body, "NEVER RETURN") || strings.Contains(body, "OLD HISTORY") {
		t.Fatal(body)
	}
	oversized := json.RawMessage(`{"ok":true,"task":{"taskId":"task","workflow":{"steps":["` + strings.Repeat("x", 25000) + `"]}}}`)
	if !strings.Contains(string(managerTaskContext(oversized, managerContextTask{TaskID: "task", Cwd: "/repo"})), "contentDeferred") {
		t.Fatal("unbounded evidence")
	}
}

func TestManagerContextRejectsUnboundedOrForgedInputBeforeCalls(t *testing.T) {
	ctx := context.WithValue(context.Background(), tokenLabelKey{}, "session:manager")
	calls := 0
	cs := connectToolClient(t, ctx, func(b *build) {
		b.caller = func(context.Context, string, any) (json.RawMessage, error) {
			calls++
			return json.RawMessage(`{}`), nil
		}
		addManagerContextTool(b)
	})
	for _, args := range []map[string]any{
		{"tasks": []managerContextTask{{"1", "/r"}, {"2", "/r"}, {"3", "/r"}, {"4", "/r"}, {"5", "/r"}}},
		{"tasks": []managerContextTask{{"same", "/r"}, {"same", "/r"}}},
		{"tasks": []managerContextTask{{"id", ""}}},
		{"callerSessionId": "foreign"},
	} {
		result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "manager_context", Arguments: args})
		if err != nil || !result.IsError || calls != 0 {
			t.Fatalf("invalid context request reached host: %v %v", result, err)
		}
	}
}

func TestComposedDispatchUsesHostCanonicalProjectPath(t *testing.T) {
	h := &workflowDispatchHub{}
	ctx, cs := workflowClient(t, h, false)
	args := dispatchArgs()
	args["cwd"] = "/repo-link"
	result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "dispatch_workflow_step", Arguments: args})
	if err != nil || result.IsError {
		t.Fatalf("canonical host path rejected: %v %v", result, err)
	}
	if h.calls[0].params["cwd"] != "/repo-link" || h.calls[1].params["cwd"] != "/repo" || h.calls[len(h.calls)-1].params["cwd"] != "/repo" {
		t.Fatal(h.calls)
	}
}

func TestComposedDispatchHonorsExplicitModelWithoutRouting(t *testing.T) {
	h := &workflowDispatchHub{}
	ctx, cs := workflowClient(t, h, false)
	args := dispatchArgs()
	args["modelSelection"] = map[string]any{"provider": "codex", "model": "gpt-6-luna", "effort": "low"}
	result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "dispatch_workflow_step", Arguments: args})
	if err != nil || result.IsError {
		t.Fatalf("%v %v", result, err)
	}
	var spawned map[string]any
	for _, call := range h.calls {
		if call.method == "routing.select" || call.method == "fleet.selectDispatchModel" {
			t.Fatal("explicit model was rerouted")
		}
		if call.method == "agents.spawn" {
			spawned = call.params
		}
	}
	for key, want := range map[string]any{"provider": "codex", "modelIdentity": "gpt-6-luna", "effort": "low", "exactModel": true, "role": "reviewer", "skipPermissions": false} {
		if spawned[key] != want {
			t.Fatalf("%s: %v != %v", key, spawned[key], want)
		}
	}
	if spawned["decisionId"] != nil || spawned["capability"] != nil {
		t.Fatal("fabricated routing provenance")
	}
	if !strings.Contains(resultText(result), `"source":"explicit"`) {
		t.Fatal(resultText(result))
	}
}

func TestExplicitModelChoiceRejectsAmbiguousOrIncompleteInputsBeforePreparing(t *testing.T) {
	for _, choice := range []map[string]any{
		{"modelSelection": map[string]any{"provider": "codex", "model": ""}},
		{"modelSelection": map[string]any{"provider": "", "model": "gpt-6-luna"}},
		{"modelSelection": map[string]any{"provider": "codex", "model": "gpt-6-luna"}, "routing": map[string]any{"provider": "claude"}},
	} {
		h := &workflowDispatchHub{}
		ctx, cs := workflowClient(t, h, false)
		args := dispatchArgs()
		for key, value := range choice {
			args[key] = value
		}
		result, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: "dispatch_workflow_step", Arguments: args})
		if err != nil || !result.IsError || len(h.calls) != 0 {
			t.Fatalf("invalid explicit choice reached host: %v %v", result, err)
		}
	}
}

func TestManagerContextIncludesBoundedFreeformAttempts(t *testing.T) {
	raw := json.RawMessage(`{"ok":true,"task":{"taskId":"manual","attempts":[{"sessionId":"old"},{"sessionId":"one"},{"sessionId":"two"},{"sessionId":"three"},{"sessionId":"current"}]},"instructions":"No pinned workflow"}`)
	result := string(managerTaskContext(raw, managerContextTask{TaskID: "manual", Cwd: "/repo"}))
	if strings.Contains(result, `"old"`) || !strings.Contains(result, `"current"`) || !strings.Contains(result, `"attemptsRemaining":1`) {
		t.Fatal(result)
	}
}
