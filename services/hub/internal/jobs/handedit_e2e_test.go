package jobs

import (
	"context"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// The end-to-end proof, and the only test here that asserts the thing the
// feature actually promises: a person edits a file in an editor and the RUNNING
// system picks it up, with nothing restarted.
//
// So it runs the real scheduler goroutine (RunScheduler, not a hand-called
// tick), against a real file at a real path, written from outside the service
// with plain os.WriteFile. Two things are faked and both are deliberate: the
// tick interval, because waiting 30 real seconds per phase is not a test, and
// the clock, because otherwise proving that a reloaded job actually FIRES means
// waiting for its trigger. Neither touches the reload path itself.

// testClock is a settable now(), safe to read from the scheduler goroutine
// while the test advances it.
type testClock struct {
	mu sync.Mutex
	t  time.Time
}

func (c *testClock) now() time.Time {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.t
}

func (c *testClock) advance(d time.Duration) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.t = c.t.Add(d)
}

// scheduleOf returns the running schedule keyed by job name, which is the only
// name a person editing the file actually chose.
func scheduleOf(s *Service) map[string]time.Time {
	out := map[string]time.Time{}
	for _, sc := range s.Schedule() {
		out[sc.Name] = sc.NextRun
	}
	return out
}

func runCount(s *Service) int {
	s.mu.Lock()
	defer s.mu.Unlock()
	n := 0
	for _, runs := range s.history {
		n += len(runs)
	}
	return n
}

func TestHandEditingTheSpecFileWorksAgainstTheRunningScheduler(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "jobs.json")
	clk := &testClock{t: time.Date(2026, 8, 24, 9, 0, 0, 0, time.UTC)}

	r := &fakeRunner{}
	s := New(nil, path, filepath.Join(dir, "jobs-history.json"), r)
	s.loc = time.UTC
	s.now = clk.now
	s.tickEvery = 10 * time.Millisecond

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go s.RunScheduler(ctx)

	// ---- 1. A job typed into the file appears in the running schedule. -----
	// No id, no timestamps: what a person writes, not what the hub writes.
	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "name": "hand added",
	      "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 10 },
	      "action": { "kind": "shell", "shell": { "command": "echo hand-added" } }
	    }
	  ]
	}`)

	waitFor(t, func() bool { return !scheduleOf(s)["hand added"].IsZero() })

	onDisk := readSpecFile(t, s)
	if len(onDisk) != 1 || onDisk[0].ID == "" {
		t.Fatalf("the hub did not complete the hand-written row on disk: %+v", onDisk)
	}
	id := onDisk[0].ID
	if want := clk.now().Add(10 * time.Minute); !scheduleOf(s)["hand added"].Equal(want) {
		t.Fatalf("next run %v, want %v", scheduleOf(s)["hand added"], want)
	}

	// ---- 2. And it fires, without anything having been restarted. ----------
	clk.advance(11 * time.Minute)
	waitFor(t, func() bool { return lastRunOf(s, id) != nil })
	if got := lastRunOf(s, id); got.Status != "ok" {
		t.Fatalf("the hand-added job failed: %+v", got)
	}
	if calls := r.recorded(); len(calls) == 0 || !strings.Contains(calls[0], "echo hand-added") {
		t.Fatalf("the command that ran was not the one written by hand: %v", calls)
	}

	// ---- 3. An edited trigger re-anchors. ---------------------------------
	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "id": "`+id+`",
	      "name": "hand added",
	      "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 240 },
	      "action": { "kind": "shell", "shell": { "command": "echo hand-added" } }
	    }
	  ]
	}`)

	want := clk.now().Add(240 * time.Minute)
	waitFor(t, func() bool { return scheduleOf(s)["hand added"].Equal(want) })

	// ---- 4. A job deleted by hand stops being scheduled, and stays stopped. -
	writeSpecFile(t, s, `{ "jobs": [] }`)
	waitFor(t, func() bool { _, ok := scheduleOf(s)["hand added"]; return !ok })

	before := runCount(s)
	// Well past every time it would have fired under either trigger.
	clk.advance(48 * time.Hour)
	// Give the scheduler many ticks to do the wrong thing.
	time.Sleep(20 * s.tickEvery)
	if after := runCount(s); after != before {
		t.Fatalf("a job deleted by hand kept firing: %d run(s) -> %d", before, after)
	}
}

// The other half of hand-editing: what a person wrote has to survive the hub
// writing the same file for its own reasons. Same real scheduler, and the hub
// write is a real RPC.
func TestExternalEditSurvivesAHubWriteAgainstTheRunningScheduler(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "jobs.json")
	clk := &testClock{t: time.Date(2026, 8, 24, 9, 0, 0, 0, time.UTC)}

	s := New(nil, path, filepath.Join(dir, "jobs-history.json"), &fakeRunner{})
	s.loc = time.UTC
	s.now = clk.now
	s.tickEvery = 10 * time.Millisecond

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go s.RunScheduler(ctx)

	fromUI := upsert(t, s, validJob("from settings"))

	writeSpecFile(t, s, `{
	  "jobs": [
	    {
	      "id": "`+fromUI.ID+`", "name": "from settings", "enabled": true,
	      "trigger": { "kind": "interval", "everyMinutes": 30 },
	      "action": { "kind": "shell", "shell": { "command": "true" } },
	      "createdAt": `+itoa(fromUI.CreatedAt)+`, "updatedAt": `+itoa(fromUI.UpdatedAt)+`
	    },
	    {
	      "id": "by-hand", "name": "by hand", "enabled": true,
	      "trigger": { "kind": "daily", "at": "03:00" },
	      "action": { "kind": "shell", "shell": { "command": "backup.sh" } }
	    }
	  ]
	}`)
	waitFor(t, func() bool { return !scheduleOf(s)["by hand"].IsZero() })

	// The hub writes, for a reason that has nothing to do with the hand edit.
	fromUI.Name = "renamed in settings"
	upsert(t, s, fromUI)

	names := map[string]bool{}
	for _, j := range readSpecFile(t, s) {
		names[j.Name] = true
	}
	if !names["by hand"] {
		t.Fatalf("the hub write clobbered the hand-added job; disk holds %v", names)
	}
	if !names["renamed in settings"] {
		t.Fatalf("the hub write did not land; disk holds %v", names)
	}
	if scheduleOf(s)["by hand"].IsZero() {
		t.Error("the hand-added job survived on disk but lost its schedule")
	}
}
