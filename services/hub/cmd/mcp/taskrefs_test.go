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

func TestTaskReferencesTransport(t *testing.T) {
	for _, tc := range []struct {
		name, label, tool, op string
		args                  map[string]any
		refuse                bool
	}{
		{"read", "session:manager", "get_task_references", "taskReferences", map[string]any{"taskId": "task", "cwd": "/project"}, false},
		{"write", "session:manager", "update_task_references", "setTaskReferences", map[string]any{"taskId": "task", "cwd": "/project", "expectedTaskRevision": 7, "upsert": []any{map[string]any{"kind": "ticket", "id": "JIRA-9"}}}, false},
		{"remove", "session:manager", "update_task_references", "setTaskReferences", map[string]any{"taskId": "task", "cwd": "/project", "expectedTaskRevision": 0, "remove": []any{map[string]any{"kind": "ticket", "id": "JIRA-9"}}}, false},
		{"no identity", "", "get_task_references", "", map[string]any{"taskId": "task", "cwd": "/project"}, true},
		{"missing CAS", "session:manager", "update_task_references", "", map[string]any{"taskId": "task", "cwd": "/project", "upsert": []any{map[string]any{"kind": "ticket", "id": "X"}}}, true},
		{"empty edit", "session:manager", "update_task_references", "", map[string]any{"taskId": "task", "cwd": "/project", "expectedTaskRevision": 1}, true},
		{"spoofed identity", "session:manager", "get_task_references", "", map[string]any{"taskId": "task", "cwd": "/project", "callerSessionId": "victim"}, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			ctx = context.WithValue(ctx, tokenLabelKey{}, tc.label)
			var got map[string]any
			cs := connectToolClient(t, ctx, func(b *build) {
				b.caller = func(_ context.Context, method string, params any) (json.RawMessage, error) {
					if method != "fleetWorkflows.request" {
						t.Errorf("unexpected method %s", method)
					}
					raw, _ := json.Marshal(params)
					_ = json.Unmarshal(raw, &got)
					return json.RawMessage(`{"ok":false,"code":"conflict","currentRevision":8,"references":{"tickets":[{"id":"HUMAN"}]}}`), nil
				}
				addTaskReferenceTools(b)
			})
			res, err := cs.CallTool(ctx, &mcp.CallToolParams{Name: tc.tool, Arguments: tc.args})
			if err != nil {
				t.Fatal(err)
			}
			if tc.refuse {
				if !res.IsError || got != nil {
					t.Fatal("invalid request reached provider")
				}
				return
			}
			if res.IsError {
				t.Fatal(resultText(res))
			}
			if got["callerSessionId"] != "manager" || got["op"] != tc.op {
				t.Fatalf("bad identity/op: %v", got)
			}
			for k, v := range tc.args {
				want, _ := json.Marshal(v)
				actual, _ := json.Marshal(got[k])
				if string(want) != string(actual) {
					t.Errorf("lost %s", k)
				}
			}
			if !strings.Contains(resultText(res), `"currentRevision":8`) || !strings.Contains(resultText(res), "HUMAN") {
				t.Fatal("provider conflict was not preserved")
			}
		})
	}
}

// The task-reference tools are a NARROW manager write. This pins the two ways
// that narrowness could silently be traded away: the tools appearing at a tier
// that has no business editing a task, and a host-only affordance (the step
// waiver, or a generic task mutation) growing a tool of its own.
func TestTaskReferenceToolScope(t *testing.T) {
	view := listToolsFor(t, authtoken.ScopeView)
	triage := listToolsFor(t, authtoken.ScopeTriage)
	operator := listToolsFor(t, authtoken.ScopeOperator)

	for _, want := range []string{"get_task_references", "update_task_references"} {
		if !operator[want] {
			t.Errorf("operator tier missing %q", want)
		}
		if view[want] || triage[want] {
			t.Errorf("%q must not reach the view/triage tiers", want)
		}
	}
	// The host user's step waiver, and generic task mutation, stay host-only:
	// no tier may hold a tool for them. editByHostUser has no bus method at all,
	// so a tool here would mean someone added one.
	for tier, tools := range map[string]map[string]bool{"view": view, "triage": triage, "operator": operator} {
		for name := range tools {
			if name == "get_task_references" || name == "update_task_references" {
				continue // the two audited, ownership-checked reference tools
			}
			for _, banned := range []string{"waive", "skip_step", "edit_task", "update_task", "set_task"} {
				if strings.Contains(name, banned) {
					t.Errorf("%s tier holds %q — host-only task edits must not become agent tools", tier, name)
				}
			}
		}
	}
	// decide_workflow_step keeps its required-review refusal; it is still the
	// only workflow-state write an agent has.
	if !operator["decide_workflow_step"] {
		t.Error("operator tier lost decide_workflow_step")
	}
}

// The help registry must actually describe the tools, or the doctrine that tells
// a manager to record a pasted PR/ticket has nowhere to land.
func TestWorkflowHelpDescribesTaskReferences(t *testing.T) {
	g := groupGuidance["workflows"]
	for _, want := range []string{
		"get_task_references",
		"update_task_references",
		"expectedTaskRevision",
		"Azure DevOps",
		"unverified references",
		"ADDITIVE",
	} {
		if !strings.Contains(g, want) {
			t.Errorf("workflows help guidance missing %q", want)
		}
	}
}
