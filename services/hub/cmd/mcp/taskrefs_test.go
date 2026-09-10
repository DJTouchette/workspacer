package main

import (
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/authtoken"
)

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
