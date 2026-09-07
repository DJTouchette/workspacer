package main

import (
	"context"
	"encoding/json"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"strings"
	"unicode/utf8"
)

const statusSummaryMethod = "agents.summarizeStatus"

type statusSummaryIn struct {
	hubArg
	SessionID string `json:"sessionId" jsonschema:"Session id from list_agents; include its hub for a remote session"`
}

// Match the router's exact unsupported-method shape. A permission error that
// merely mentions an unknown method must remain an error, never degradation.
func summaryUnavailableError(message, method string) bool {
	if strings.HasPrefix(method, "hub:") {
		peer := strings.SplitN(strings.TrimPrefix(method, "hub:"), "/", 2)[0]
		message = strings.TrimPrefix(message, "hub:"+peer+": ")
	}
	return message == "no provider for "+method || message == "no provider for "+statusSummaryMethod ||
		message == "unknown method: "+method || message == "unknown method: "+statusSummaryMethod
}
func unavailableSummary(reason string) string {
	raw, _ := json.Marshal(map[string]any{
		"contract": "agent-status-summary/v1", "status": "unavailable", "reason": reason,
		"activity": nil, "progress": nil, "blocker": nil, "nextStep": nil,
		"earliestRetainedTask": nil, "latestExplicitProgress": nil, "source": nil,
		"provider": nil, "model": nil, "cached": false, "unknowns": []string{reason},
	})
	return string(raw)
}
func addStatusSummaryTool(b *build) {
	if !b.allowed(statusSummaryMethod) {
		return
	}
	const name = "summarize_agent_status"
	const desc = "On demand: summarize what a visible agent is doing from bounded task and recent text. Uses the source hub's configured CLI provider/model; requires a compatible desktop and daemon there. Availability status is not worker lifecycle. Never poll this tool or substitute it for direct blocked/completion evidence or the final worker report. Include hub from list_agents for remote sessions."
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: statusSummaryMethod, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc}, func(ctx context.Context, _ *mcp.CallToolRequest, in statusSummaryIn) (*mcp.CallToolResult, any, error) {
		method := statusSummaryMethod
		if peer := in.takeHub(); peer != "" {
			method = "hub:" + peer + "/" + method
		}
		res, err := b.c.Call(ctx, method, in)
		text := string(res)
		if err != nil {
			if summaryUnavailableError(err.Error(), method) {
				// No desktop and an old desktop are indistinguishable here.
				text = unavailableSummary("desktop-summary-unavailable")
			} else {
				// Do not expose provider stderr or disguise permission denial.
				lower := strings.ToLower(err.Error())
				denied := false
				for _, marker := range []string{"permission", "denied", "not allowed", "forbidden", "may not call", "scope"} {
					denied = denied || strings.Contains(lower, marker)
				}
				if denied {
					return &mcp.CallToolResult{IsError: true, Content: []mcp.Content{&mcp.TextContent{Text: "permission denied"}}}, nil, nil
				}
				text = unavailableSummary("source-unavailable")
			}
		}
		// Fail closed on unknown/old provider JSON. No raw transcript passthrough.
		if !validSummaryResponse(text) {
			text = unavailableSummary("invalid-summary-response")
		}
		return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: text}}}, nil, nil
	})
}

// A peer must speak the compact host contract, never append arbitrary source.
func validSummaryResponse(text string) bool {
	if len(text) > 12000 || utf8.RuneCountInString(text) > 3000 {
		return false
	}
	var value map[string]json.RawMessage
	if json.Unmarshal([]byte(text), &value) != nil || len(value) != 14 {
		return false
	}
	readString := func(key string, limit int, nullable bool) bool {
		raw, ok := value[key]
		if !ok {
			return false
		}
		if string(raw) == "null" {
			return nullable
		}
		var s string
		return json.Unmarshal(raw, &s) == nil && utf8.RuneCountInString(s) <= limit
	}
	if string(value["contract"]) != `"agent-status-summary/v1"` {
		return false
	}
	status := string(value["status"])
	if status != `"ok"` && status != `"disabled"` && status != `"unavailable"` {
		return false
	}
	for _, key := range []string{"activity", "progress", "blocker", "nextStep", "earliestRetainedTask", "latestExplicitProgress"} {
		if !readString(key, 240, true) {
			return false
		}
	}
	if !readString("provider", 80, true) || !readString("model", 120, true) || !readString("reason", 80, true) {
		return false
	}
	var cached bool
	if json.Unmarshal(value["cached"], &cached) != nil || string(value["cached"]) == "null" {
		return false
	}
	var unknowns []string
	if json.Unmarshal(value["unknowns"], &unknowns) != nil || unknowns == nil || len(unknowns) > 5 {
		return false
	}
	for _, s := range unknowns {
		if utf8.RuneCountInString(s) > 240 {
			return false
		}
	}
	if string(value["source"]) != "null" {
		var source map[string]json.RawMessage
		if json.Unmarshal(value["source"], &source) != nil || len(source) != 6 {
			return false
		}
		for _, key := range []string{"throughSeq", "firstSeq"} {
			var n uint64
			if json.Unmarshal(source[key], &n) != nil || string(source[key]) == "null" {
				return false
			}
		}
		for _, key := range []string{"headTruncated", "tailTruncated", "textTruncated"} {
			var b bool
			if json.Unmarshal(source[key], &b) != nil || string(source[key]) == "null" {
				return false
			}
		}
		if string(source["timestamp"]) != "null" {
			var stamp string
			if json.Unmarshal(source["timestamp"], &stamp) != nil || len(stamp) > 40 {
				return false
			}
		}
	}
	return true
}
