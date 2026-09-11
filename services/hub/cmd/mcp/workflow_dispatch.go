package main

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/routing"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// No model, role, parent, template or arbitrary spawn arguments: those are
// derived from the owned pinned step, then passed through spawnWithGrants.
type dispatchWorkflowIn struct {
	TaskID               string             `json:"taskId" jsonschema:"the pinned task you own"`
	Cwd                  string             `json:"cwd" jsonschema:"that task's local project directory"`
	StepID               string             `json:"stepId" jsonschema:"exact intended step; a retry never advances to a different step"`
	ExpectedTaskRevision int                `json:"expectedTaskRevision" jsonschema:"current task revision; stale calls are refused before dispatch"`
	TemplateParams       map[string]string  `json:"templateParams,omitempty" jsonschema:"task-specific inputs named by the pinned step; no host cwd/projectCwd overrides"`
	Label                string             `json:"label,omitempty" jsonschema:"short worker label"`
	Run                  *bool              `json:"run,omitempty" jsonschema:"optional explicit conditional-step decision; false records a skip and stops without dispatching the next step"`
	Reason               string             `json:"reason,omitempty" jsonschema:"required with run"`
	ExecutionTarget      string             `json:"executionTarget,omitempty" jsonschema:"paired for an explicitly selected paired worker; omit for local"`
	RemoteCwd            string             `json:"remoteCwd,omitempty" jsonschema:"exact remote path from list_dispatch_targets; required with paired"`
	ProfileID            string             `json:"profileId,omitempty" jsonschema:"optional granted Claude profile"`
	SkipPermissions      *bool              `json:"skipPermissions,omitempty" jsonschema:"optional permission request; existing token grants still bind"`
	Routing              *workflowRoutingIn `json:"routing,omitempty" jsonschema:"optional routing constraints; role/cwd/previousProvider are host-derived"`
	WatchContextUsedPct  *float64           `json:"watchContextUsedPct,omitempty" jsonschema:"optionally arm one local active-context wake after spawning (0–100]; a watch failure never retries the spawn"`
}

type workflowRoutingIn struct {
	Provider                     string          `json:"provider,omitempty" jsonschema:"optional provider pin; unavailable routes never substitute silently"`
	Profile                      string          `json:"profile,omitempty" jsonschema:"optional routing profile"`
	Difficulty                   string          `json:"difficulty,omitempty"`
	Risk                         string          `json:"risk,omitempty"`
	DecisionDensity              string          `json:"decisionDensity,omitempty"`
	RequireIndependentFamily     bool            `json:"requireIndependentFamily,omitempty"`
	ForecastDemandBeforeResetPct *float64        `json:"forecastDemandBeforeResetPct,omitempty"`
	ExpectedWork                 []routingWorkIn `json:"expectedWork,omitempty"`
}

type workflowDispatchPlan struct {
	TaskID               string `json:"taskId"`
	Cwd                  string `json:"cwd"`
	StepID               string `json:"stepId"`
	ExpectedTaskRevision int    `json:"expectedTaskRevision"`
	Role                 string `json:"role"`
	Stage                string `json:"stage"`
	Template             string `json:"template"`
	ToolScope            string `json:"toolScope"`
	AfterDispatchID      string `json:"afterDispatchId"`
	PreviousProvider     string `json:"previousProvider"`
}

func addWorkflowDispatchTool(b *build) {
	for _, method := range []string{"fleetWorkflows.request", "routing.select", "fleet.selectDispatchModel", "agents.spawn", "config.get", "agents.sendMessage", "agents.notifyWhen"} {
		if !b.allowed(method) {
			return
		}
	}
	const name = "dispatch_workflow_step"
	const desc = "Dispatch one exact pinned step: validate/decide, route, apply host metadata and spawn in one call. Optional context watch. Returns a compact receipt; never retries or launches a following step."
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: "agents.spawn", Group: "workflows"})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc}, func(ctx context.Context, _ *mcp.CallToolRequest, in dispatchWorkflowIn) (*mcp.CallToolResult, any, error) {
		return dispatchWorkflow(ctx, b, in)
	})
}

func dispatchWorkflow(ctx context.Context, b *build, in dispatchWorkflowIn) (*mcp.CallToolResult, any, error) {
	caller := callerSessionID(ctx)
	if caller == "" {
		return toolError("Workflow dispatch requires an authenticated local manager")
	}
	if strings.TrimSpace(in.TaskID) == "" || strings.TrimSpace(in.Cwd) == "" || strings.TrimSpace(in.StepID) == "" || in.ExpectedTaskRevision < 0 {
		return toolError("taskId, cwd, stepId and a nonnegative expectedTaskRevision are required")
	}
	if in.ExecutionTarget != "" && in.ExecutionTarget != "paired" {
		return toolError("executionTarget must be paired or omitted")
	}
	if (in.ExecutionTarget == "paired") != (strings.TrimSpace(in.RemoteCwd) != "") {
		return toolError("paired requires remoteCwd; local dispatch must omit it")
	}
	if in.Run != nil && strings.TrimSpace(in.Reason) == "" {
		return toolError("run requires a reason")
	}
	if in.WatchContextUsedPct != nil {
		n := *in.WatchContextUsedPct
		if math.IsNaN(n) || math.IsInf(n, 0) || n <= 0 || n > 100 || in.ExecutionTarget != "" {
			return toolError("watchContextUsedPct must be in (0,100] and is local-only")
		}
	}
	// Fail granted-profile requests before recording a conditional decision.
	if in.ProfileID != "" {
		found := false
		for _, profile := range b.profiles {
			if profile == in.ProfileID {
				found = true
			}
		}
		if !found {
			return toolError("profile is not granted to this session")
		}
	}
	prepare := map[string]any{"op": "prepareDispatch", "callerSessionId": caller, "taskId": in.TaskID, "cwd": in.Cwd, "stepId": in.StepID, "expectedTaskRevision": in.ExpectedTaskRevision, "templateParams": in.TemplateParams}
	if in.Run != nil {
		prepare["run"] = *in.Run
		prepare["reason"] = in.Reason
	}
	raw, err := b.call(ctx, "fleetWorkflows.request", prepare)
	if err != nil {
		return fleetComposeError("prepare", err.Error(), false)
	}
	var prepared struct {
		OK       bool                  `json:"ok"`
		Skipped  bool                  `json:"skipped"`
		Dispatch *workflowDispatchPlan `json:"dispatch"`
	}
	if json.Unmarshal(raw, &prepared) != nil {
		return fleetComposeError("prepare", "Host returned an unreadable dispatch plan", false)
	}
	if !prepared.OK || prepared.Skipped {
		return fleetRawResult(raw, !prepared.OK)
	}
	// The host canonicalizes cwd before its ownership check; use that path
	// rather than rejecting a valid symlink or platform-specific spelling.
	plan := prepared.Dispatch
	if plan == nil || plan.TaskID != in.TaskID || strings.TrimSpace(plan.Cwd) == "" || plan.StepID != in.StepID || plan.ExpectedTaskRevision < in.ExpectedTaskRevision || plan.Role == "" || plan.Stage == "" || plan.Template == "" || (plan.ToolScope != "view" && plan.ToolScope != "operator") {
		return fleetComposeError("prepare", "Host returned an incomplete or mismatched dispatch plan", false)
	}
	route := routingSelectIn{Role: plan.Role, Cwd: plan.Cwd, PreviousProvider: plan.PreviousProvider, ProfileID: in.ProfileID, TicketID: plan.TaskID + ":" + plan.StepID}
	if r := in.Routing; r != nil {
		route.Provider = r.Provider
		route.Profile = r.Profile
		route.Difficulty = r.Difficulty
		route.Risk = r.Risk
		route.DecisionDensity = r.DecisionDensity
		route.RequireIndependentFamily = r.RequireIndependentFamily
		route.ForecastDemandBeforeResetPct = r.ForecastDemandBeforeResetPct
		route.ExpectedWork = r.ExpectedWork
	}
	method := "routing.select"
	if in.ExecutionTarget == "paired" {
		method = "fleet.selectDispatchModel"
		route.Cwd = in.RemoteCwd
	}
	raw, err = b.call(ctx, method, route)
	if err != nil {
		return fleetComposeError("routing", err.Error(), false)
	}
	var decision routing.Decision
	var eligibility struct {
		Eligible *bool `json:"eligible"`
	}
	if json.Unmarshal(raw, &eligibility) != nil || eligibility.Eligible == nil {
		return fleetComposeError("routing", "Router returned no eligibility decision: "+string(raw), false)
	}
	if json.Unmarshal(raw, &decision) != nil {
		return fleetComposeError("routing", "Router returned an unreadable decision", false)
	}
	if !decision.Eligible {
		return fleetValueResult(map[string]any{"ok": false, "phase": "routing", "admitted": false, "routing": routeReceipt(decision)}, true)
	}
	if decision.Provider == "" || decision.Model == "" || decision.Capability == "" || decision.DecisionID == "" || decision.Role != plan.Role || (route.RequireIndependentFamily && !decision.IndependentFamily) {
		return fleetComposeError("routing", "Router returned an incomplete or mismatched eligible decision", false)
	}
	spawn := spawnAgentIn{
		Cwd: plan.Cwd, TaskID: plan.TaskID, WorkflowStepID: plan.StepID, ExpectedTaskRevision: &plan.ExpectedTaskRevision,
		Stage: plan.Stage, Role: plan.Role, AfterDispatchID: plan.AfterDispatchID, ParentSessionId: caller,
		Template: plan.Template, TemplateParams: in.TemplateParams, ToolScope: plan.ToolScope,
		Provider: decision.Provider, Model: decision.Model, Effort: decision.Effort, Capability: decision.Capability, DecisionID: decision.DecisionID,
		Label: in.Label, ProfileID: in.ProfileID, SkipPermissions: in.SkipPermissions, ExecutionTarget: in.ExecutionTarget, RemoteCwd: in.RemoteCwd,
	}
	result, _, err := spawnWithGrants(ctx, b, "agents.spawn", spawn)
	if err != nil {
		return fleetComposeError("spawn", err.Error(), true)
	}
	if result == nil {
		return fleetComposeError("spawn", "No spawn receipt", true)
	}
	if result.IsError {
		return fleetComposeError("spawn", resultText(result), true)
	}
	var receipt map[string]json.RawMessage
	if json.Unmarshal([]byte(resultText(result)), &receipt) != nil || receipt == nil {
		return fleetComposeError("spawn", "Unreadable spawn receipt: "+resultText(result), true)
	}
	if in.ExecutionTarget == "" && sessionIDFrom(result) == "" {
		return fleetComposeError("spawn", "No local sessionId in spawn receipt: "+resultText(result), true)
	}
	// Preserve every host warning, clamp, uncertain-admission and delivery field.
	// Only the rendered copy of the worker prompt is omitted.
	if _, ok := receipt["renderedMessage"]; ok {
		delete(receipt, "renderedMessage")
		receipt["renderedMessageOmitted"] = json.RawMessage("true")
	}
	receipt["selection"], _ = json.Marshal(routeReceipt(decision))
	if in.WatchContextUsedPct != nil {
		id := sessionIDFrom(result)
		if id == "" {
			receipt["watchError"], _ = json.Marshal("No confirmed sessionId; inspect admission before arming a watch. Do not repeat the spawn.")
		} else {
			watch, watchErr := b.call(ctx, "agents.notifyWhen", notifyWhenIn{SessionID: id, NotifySessionID: caller, ContextUsedPct: *in.WatchContextUsedPct})
			if watchErr != nil {
				receipt["watchError"], _ = json.Marshal(watchErr.Error() + "; the worker was spawned. Retry only notify_when.")
			} else {
				receipt["watch"] = watch
			}
		}
	}
	return fleetValueResult(receipt, false)
}

func routeReceipt(d routing.Decision) map[string]any {
	return map[string]any{"decisionId": d.DecisionID, "eligible": d.Eligible, "provider": d.Provider, "model": d.Model, "effort": d.Effort, "capability": d.Capability, "mode": d.Mode, "independentFamily": d.IndependentFamily, "reason": d.Reason}
}
func fleetComposeError(phase, message string, uncertain bool) (*mcp.CallToolResult, any, error) {
	note := "No worker was dispatched; an explicit conditional decision may already be recorded. Refresh task state before retrying."
	if uncertain {
		note = "Spawn admission or delivery may be uncertain. Do not repeat this call; inspect manager_context, list_agents and list_dispatches to reconcile the existing worker first."
	}
	return fleetValueResult(map[string]any{"ok": false, "phase": phase, "error": message, "admissionUncertain": uncertain, "instructions": note}, true)
}
func fleetValueResult(value any, isError bool) (*mcp.CallToolResult, any, error) {
	raw, err := json.Marshal(value)
	if err != nil {
		return nil, nil, fmt.Errorf("render fleet result: %w", err)
	}
	return fleetRawResult(raw, isError)
}
func fleetRawResult(raw json.RawMessage, isError bool) (*mcp.CallToolResult, any, error) {
	return &mcp.CallToolResult{IsError: isError, Content: []mcp.Content{&mcp.TextContent{Text: string(raw)}}}, nil, nil
}
