package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

// agents.spawn's `message` — the new agent's FIRST PROMPT, carried by the spawn
// instead of a follow-up agents.sendMessage.
//
// The brain MIRRORS this param rather than declining it (unlike resultSchema,
// whose two halves are both desktop-owned): the field is claudemon's, and the
// brain reaches the same two claudemon spawn routes the desktop does. Declining
// it would mean a bus caller's dispatch silently loses its task whenever the
// desktop is not running — a worker running with no instructions, which is the
// failure this whole field exists to prevent.

func TestSpawnCarriesTheFirstMessageOnTheManagedPayload(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	params := []byte(`{"cwd":"/tmp","provider":"codex","transport":"stream","message":"ship the thing"}`)
	res, err := reg.handle(context.Background(), "agents.spawn", params)
	if err != nil {
		t.Fatal(err)
	}

	managed := rec.calls("/sessions/spawn-managed")
	if len(managed) != 1 {
		t.Fatalf("expected one spawn-managed call, got %d", len(managed))
	}
	if got := managed[0].body["first_message"]; got != "ship the thing" {
		t.Errorf("first_message = %v, want the dispatch text — without it the worker starts with no task", got)
	}
	// It must be its OWN field, never folded into `instructions`: instructions
	// is a passive prefix for host contracts and never starts a turn, so a
	// dispatch put there waits forever for the prompt it is.
	instructions, _ := managed[0].body["instructions"].(string)
	if !strings.Contains(instructions, "wks-escalation") || strings.Contains(instructions, "ship the thing") {
		t.Errorf("instructions = %q, want escalation contract without the dispatch", instructions)
	}
	assertMessageQueued(t, res, true)
}

func TestSpawnCarriesTheFirstMessageOnThePTYPayload(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	params := []byte(`{"cwd":"/tmp","transport":"pty","message":"ship the thing"}`)
	res, err := reg.handle(context.Background(), "agents.spawn", params)
	if err != nil {
		t.Fatal(err)
	}

	spawns := rec.calls("/sessions/spawn")
	if len(spawns) != 1 {
		t.Fatalf("expected one PTY spawn call, got %d", len(spawns))
	}
	if got := spawns[0].body["first_message"]; got != "ship the thing" {
		t.Errorf("first_message = %v, want the dispatch text", got)
	}
	assertMessageQueued(t, res, true)
}

// The queued flag is READ BACK from the daemon, never assumed. A daemon that
// predates the field answers a perfectly normal spawn with the prompt nowhere;
// reporting `messageQueued: true` on that would tell the dispatcher a task
// landed when it did not, and an idle worker is indistinguishable from a wedged
// one. False is what makes the facade's fallback send fire.
func TestMessageQueuedReportsTheDaemonsAnswerNotTheRequest(t *testing.T) {
	rec := newRecorder()
	srv := rec.serverWithoutFirstMessageSupport()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	res, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","transport":"pty","message":"ship the thing"}`))
	if err != nil {
		t.Fatal(err)
	}
	assertMessageQueued(t, res, false)
}

// No message asked for, no claim made: the result shape stays byte-for-byte
// what every other spawn has always answered.
func TestSpawnWithoutAMessageMakesNoDeliveryClaim(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	res, err := reg.handle(context.Background(), "agents.spawn", []byte(`{"cwd":"/tmp","transport":"pty"}`))
	if err != nil {
		t.Fatal(err)
	}
	var out map[string]any
	if err := json.Unmarshal(res, &out); err != nil {
		t.Fatal(err)
	}
	if _, ok := out["messageQueued"]; ok {
		t.Errorf("result = %s, want no messageQueued key when no message was sent", res)
	}
	if out["sessionId"] == "" || out["sessionId"] == nil {
		t.Errorf("result = %s, want a sessionId", res)
	}
}

func assertMessageQueued(t *testing.T, res json.RawMessage, want bool) {
	t.Helper()
	var out struct {
		SessionID     string `json:"sessionId"`
		MessageQueued bool   `json:"messageQueued"`
	}
	if err := json.Unmarshal(res, &out); err != nil {
		t.Fatalf("result %s: %v", res, err)
	}
	if out.SessionID == "" {
		t.Errorf("result = %s, want a sessionId", res)
	}
	if out.MessageQueued != want {
		t.Errorf("messageQueued = %v, want %v (result %s)", out.MessageQueued, want, res)
	}
}
