package main

import (
	"context"
	"strings"
	"testing"
)

// agents.spawn's `template`/`templateParams` render a library dispatch item
// into the new agent's first message — but ONLY on the desktop
// (hubCapabilities.ts): rendering also compiles the template's default
// resultSchema, and that machinery is itself desktop-owned and declined here
// (see spawnParamsDeclined in parity_test.go). spawnParams therefore carries
// no field for either key, and before this test, unmarshalling the params
// silently dropped whichever of them a caller sent: the spawn proceeded with
// an EMPTY first message, so the new session started, received no task, and
// exited immediately — while this capability still answered a normal-looking
// success (`sessionId`, no error, no `messageQueued`) with no way for the
// caller to tell it apart from an ordinary quiet worker. A manager that
// dispatched with a template got back a plausible id and waited forever for a
// completion wake that could never arrive.
//
// The fix refuses the spawn outright instead of silently honoring half of it.

func TestSpawnRefusesTemplate(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	_, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","template":"scout-task","templateParams":{"task":"investigate the thing"}}`))
	if err == nil {
		t.Fatal("expected agents.spawn to refuse a template param, got no error")
	}
	if !strings.Contains(err.Error(), "template") || !strings.Contains(err.Error(), "desktop-only") {
		t.Errorf("error = %q, want it to name template as desktop-only", err.Error())
	}

	if calls := rec.calls("/sessions/spawn"); len(calls) != 0 {
		t.Errorf("expected no PTY spawn to reach claudemon, got %d — a refused spawn must not start a session with no task", len(calls))
	}
	if calls := rec.calls("/sessions/spawn-managed"); len(calls) != 0 {
		t.Errorf("expected no managed spawn to reach claudemon, got %d", len(calls))
	}
}

func TestSpawnRefusesTemplateParamsWithoutTemplate(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	_, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","templateParams":{"task":"x"}}`))
	if err == nil {
		t.Fatal("expected agents.spawn to refuse a bare templateParams, got no error")
	}
	if !strings.Contains(err.Error(), "templateParams") {
		t.Errorf("error = %q, want it to name templateParams", err.Error())
	}
}

// A spawn that composes its own first message, the supported path, must be
// untouched by the guard above.
func TestSpawnWithPlainMessageStillWorks(t *testing.T) {
	rec := newRecorder()
	srv := rec.server()
	defer srv.Close()
	reg := newSpawnTestRegistry(t, srv.URL)

	res, err := reg.handle(context.Background(), "agents.spawn",
		[]byte(`{"cwd":"/tmp","transport":"pty","message":"do the task"}`))
	if err != nil {
		t.Fatalf("plain message spawn should succeed, got error: %v", err)
	}
	assertMessageQueued(t, res, true)
}
