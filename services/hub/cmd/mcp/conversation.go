// get_conversation's opt-in payload reductions.
//
// A session's conversation snapshot can run to megabytes of tool calls, tool
// results and usage blobs — far more than a fleet manager needs when its
// question is "what did this worker report". These reductions run FACADE-side,
// on the provider's JSON result, so they work identically for local sessions,
// federated peers (hub:<peer>/), and every provider shape that follows the
// { seq, items } convention:
//
//   - lastMessage: reduce to just the worker's FINAL assistant message — the
//     trailing run of assistant_text items (usage/plan metadata skipped),
//     joined in order. Returns { seq, lastMessage } instead of an item stream.
//   - textOnly: keep only user_message / assistant_text items, stripping
//     tool_use / tool_result / usage / plan / command noise. Same { seq,
//     items } envelope, so sinceSeq tracking is unchanged.
//
// Both compose with sinceSeq (the provider windows the items first; reduction
// applies to the returned window). When both are set, lastMessage wins — it is
// the stronger reduction. A result that doesn't parse as the expected envelope
// passes through untouched (fail open: an odd provider shape degrades to the
// unreduced payload, never to an error).
package main

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// addConversationTool is addHubTool specialized to get_conversation: the two
// reduction params are FACADE-local (stripped before the call goes to the
// provider, like hubArg's hub), and the provider's result is reduced on the
// way back.
func addConversationTool(b *build, name, desc, method string) {
	if !b.allowed(method) {
		return
	}
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: method, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc},
		func(ctx context.Context, _ *mcp.CallToolRequest, in conversationIn) (*mcp.CallToolResult, any, error) {
			lastMessage, textOnly := in.LastMessage, in.TextOnly
			in.LastMessage, in.TextOnly = false, false
			m := method
			if peer := in.takeHub(); peer != "" {
				m = "hub:" + peer + "/" + method
			}
			res, err := b.c.Call(ctx, m, in)
			if err != nil {
				return &mcp.CallToolResult{
					IsError: true,
					Content: []mcp.Content{&mcp.TextContent{Text: err.Error()}},
				}, nil, nil
			}
			text := string(reduceConversation(res, lastMessage, textOnly))
			if text == "" || text == "null" {
				text = "ok"
			}
			return &mcp.CallToolResult{
				Content: []mcp.Content{&mcp.TextContent{Text: text}},
			}, nil, nil
		})
}

// conversationSnap is the { seq, items } envelope every provider's
// sessions.conversation result follows (claudemon's conversation snapshot).
// Seq is a pointer purely for envelope DETECTION: a result with no seq field
// at all is some other shape and must pass through unreduced — unknown JSON
// fields unmarshal silently, so field presence is the only reliable signal.
type conversationSnap struct {
	Seq   *uint64           `json:"seq"`
	Items []json.RawMessage `json:"items"`
}

// itemKind reads one conversation item's discriminator ("" when unreadable).
func itemKind(item json.RawMessage) string {
	var k struct {
		Kind string `json:"kind"`
	}
	if json.Unmarshal(item, &k) != nil {
		return ""
	}
	return k.Kind
}

// reduceConversation applies the opt-in reductions to a provider's
// sessions.conversation result. No reduction requested, or a result that
// doesn't parse as the { seq, items } envelope, passes through byte-for-byte.
func reduceConversation(raw json.RawMessage, lastMessage, textOnly bool) json.RawMessage {
	if !lastMessage && !textOnly {
		return raw
	}
	var snap conversationSnap
	if err := json.Unmarshal(raw, &snap); err != nil || snap.Seq == nil {
		return raw
	}

	if lastMessage {
		out := map[string]any{"seq": *snap.Seq}
		if text, ok := trailingAssistantText(snap.Items); ok {
			out["lastMessage"] = text
		} else {
			out["lastMessage"] = nil
			out["note"] = "no assistant message in range (the session may not have replied yet, or sinceSeq is past its last reply)"
		}
		if b, err := json.Marshal(out); err == nil {
			return b
		}
		return raw
	}

	// textOnly: keep only the user/assistant text turns.
	kept := make([]json.RawMessage, 0, len(snap.Items))
	for _, item := range snap.Items {
		switch itemKind(item) {
		case "user_message", "assistant_text":
			kept = append(kept, item)
		}
	}
	b, err := json.Marshal(map[string]any{"seq": *snap.Seq, "items": kept})
	if err != nil {
		return raw
	}
	return b
}

// trailingAssistantText joins the trailing run of assistant_text items — the
// final assistant message. Metadata items (usage, plan) ride between and after
// a message's text blocks, so they are skipped without ending the run; any
// other kind (a user turn, tool activity, command output) does end it. Blocks
// are returned in original order, joined with blank lines.
func trailingAssistantText(items []json.RawMessage) (string, bool) {
	var blocks []string
	for i := len(items) - 1; i >= 0; i-- {
		switch itemKind(items[i]) {
		case "usage", "plan":
			continue
		case "assistant_text":
			var t struct {
				Text string `json:"text"`
			}
			if json.Unmarshal(items[i], &t) == nil && t.Text != "" {
				blocks = append(blocks, t.Text)
			}
			continue
		}
		break
	}
	if len(blocks) == 0 {
		return "", false
	}
	// Collected back-to-front; restore original order.
	for l, r := 0, len(blocks)-1; l < r; l, r = l+1, r-1 {
		blocks[l], blocks[r] = blocks[r], blocks[l]
	}
	return strings.Join(blocks, "\n\n"), true
}
