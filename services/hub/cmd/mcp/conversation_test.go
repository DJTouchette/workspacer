package main

import (
	"encoding/json"
	"strings"
	"testing"
)

// item builds one conversation item as raw JSON.
func item(t *testing.T, fields map[string]any) json.RawMessage {
	t.Helper()
	b, err := json.Marshal(fields)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// snap builds a { seq, items } provider result.
func snap(t *testing.T, seq uint64, items ...json.RawMessage) json.RawMessage {
	t.Helper()
	b, err := json.Marshal(map[string]any{"seq": seq, "items": items})
	if err != nil {
		t.Fatal(err)
	}
	return b
}

// A realistic windowed conversation: task turn, tool activity, then the final
// report streamed as two text blocks with usage metadata riding between.
func fixtureItems(t *testing.T) []json.RawMessage {
	return []json.RawMessage{
		item(t, map[string]any{"kind": "user_message", "text": "fix the tests"}),
		item(t, map[string]any{"kind": "assistant_text", "text": "on it"}),
		item(t, map[string]any{"kind": "tool_use", "id": "t1", "name": "Bash", "input": map[string]any{"command": "npm test"}}),
		item(t, map[string]any{"kind": "tool_result", "tool_use_id": "t1", "content": strings.Repeat("x", 5000), "is_error": false}),
		item(t, map[string]any{"kind": "assistant_text", "text": "All 42 tests pass."}),
		item(t, map[string]any{"kind": "usage", "usage": map[string]any{"output_tokens": 12}}),
		item(t, map[string]any{"kind": "assistant_text", "text": "Merged to master."}),
		item(t, map[string]any{"kind": "usage", "usage": map[string]any{"output_tokens": 3}}),
	}
}

func TestReduceConversationPassthrough(t *testing.T) {
	raw := snap(t, 8, fixtureItems(t)...)
	if got := reduceConversation(raw, false, false); string(got) != string(raw) {
		t.Fatalf("no-flag call must pass through unchanged, got %s", got)
	}
	// A result that isn't the { seq, items } envelope also passes through.
	odd := json.RawMessage(`{"messages":["hi"]}`)
	if got := reduceConversation(odd, true, true); string(got) != string(odd) {
		t.Fatalf("non-envelope result must pass through, got %s", got)
	}
}

func TestReduceConversationLastMessage(t *testing.T) {
	got := reduceConversation(snap(t, 8, fixtureItems(t)...), true, false)
	var out struct {
		Seq         uint64  `json:"seq"`
		LastMessage *string `json:"lastMessage"`
	}
	if err := json.Unmarshal(got, &out); err != nil {
		t.Fatal(err)
	}
	if out.Seq != 8 {
		t.Fatalf("seq must survive reduction, got %d", out.Seq)
	}
	if out.LastMessage == nil {
		t.Fatal("expected a lastMessage")
	}
	// The trailing run: both final text blocks, in order, usage skipped — and
	// NOT the pre-tool "on it" (a different message, ended by tool activity).
	want := "All 42 tests pass.\n\nMerged to master."
	if *out.LastMessage != want {
		t.Fatalf("lastMessage = %q, want %q", *out.LastMessage, want)
	}
	if strings.Contains(string(got), "tool_result") || strings.Contains(string(got), strings.Repeat("x", 100)) {
		t.Fatal("lastMessage reduction must drop tool payloads")
	}
}

// Composes with sinceSeq: the provider windows the items; a window past the
// last reply (or before any reply) reduces to an explicit "nothing here",
// never to a fabricated message.
func TestReduceConversationLastMessageEmptyWindow(t *testing.T) {
	got := reduceConversation(snap(t, 12), true, false)
	var out map[string]any
	if err := json.Unmarshal(got, &out); err != nil {
		t.Fatal(err)
	}
	if v, present := out["lastMessage"]; !present || v != nil {
		t.Fatalf("empty window must yield lastMessage:null, got %s", got)
	}
	if note, _ := out["note"].(string); !strings.Contains(note, "no assistant message") {
		t.Fatalf("empty window must carry an explanatory note, got %s", got)
	}
	// A window whose only content is tool activity behaves the same.
	toolOnly := snap(t, 13,
		item(t, map[string]any{"kind": "tool_use", "id": "t2", "name": "Bash", "input": map[string]any{}}),
		item(t, map[string]any{"kind": "tool_result", "tool_use_id": "t2", "content": "ok", "is_error": false}),
	)
	var out2 map[string]any
	if err := json.Unmarshal(reduceConversation(toolOnly, true, false), &out2); err != nil {
		t.Fatal(err)
	}
	if out2["lastMessage"] != nil {
		t.Fatalf("tool-only window must yield lastMessage:null, got %v", out2)
	}
}

func TestReduceConversationTextOnly(t *testing.T) {
	got := reduceConversation(snap(t, 8, fixtureItems(t)...), false, true)
	var out conversationSnap
	if err := json.Unmarshal(got, &out); err != nil {
		t.Fatal(err)
	}
	if out.Seq == nil || *out.Seq != 8 {
		t.Fatalf("seq must survive reduction, got %v", out.Seq)
	}
	kinds := make([]string, 0, len(out.Items))
	for _, it := range out.Items {
		kinds = append(kinds, itemKind(it))
	}
	want := []string{"user_message", "assistant_text", "assistant_text", "assistant_text"}
	if strings.Join(kinds, ",") != strings.Join(want, ",") {
		t.Fatalf("textOnly kinds = %v, want %v", kinds, want)
	}
	// Empty windows keep the envelope with an empty array, not null.
	empty := reduceConversation(snap(t, 3), false, true)
	if !strings.Contains(string(empty), `"items":[]`) {
		t.Fatalf("empty textOnly window must keep items:[], got %s", empty)
	}
}

func TestReduceConversationLastMessageWinsOverTextOnly(t *testing.T) {
	got := reduceConversation(snap(t, 8, fixtureItems(t)...), true, true)
	if !strings.Contains(string(got), `"lastMessage"`) || strings.Contains(string(got), `"items"`) {
		t.Fatalf("lastMessage must win when both reductions are set, got %s", got)
	}
}

// The reduction params are facade-local: once cleared (as the tool handler
// does before forwarding), they must vanish from the wire so providers never
// see stray fields.
func TestConversationInStripsFacadeParams(t *testing.T) {
	in := conversationIn{SessionID: "s1", LastMessage: true, TextOnly: true}
	in.LastMessage, in.TextOnly = false, false
	b, err := json.Marshal(in)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(b), "lastMessage") || strings.Contains(string(b), "textOnly") {
		t.Fatalf("cleared facade params must be omitted from forwarded params, got %s", b)
	}
}
