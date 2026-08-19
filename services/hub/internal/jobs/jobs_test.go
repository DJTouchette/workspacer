package jobs

import (
	"context"
	"encoding/json"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

// fakeRunner records calls and can be told to fail.
type fakeRunner struct {
	mu     sync.Mutex
	calls  []string // "method" or "shell:<cmd>"
	params []any
	fail   bool
}

func (f *fakeRunner) Call(_ context.Context, method string, params any) (json.RawMessage, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, method)
	f.params = append(f.params, params)
	if f.fail {
		return nil, context.DeadlineExceeded
	}
	if method == "agents.spawn" {
		return json.RawMessage(`{"sessionId":"sess-1"}`), nil
	}
	return json.RawMessage(`{"ok":true}`), nil
}

func (f *fakeRunner) Shell(_ context.Context, command, _ string) (string, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, "shell:"+command)
	return "line1\nline2", nil
}

func (f *fakeRunner) recorded() []string {
	f.mu.Lock()
	defer f.mu.Unlock()
	return append([]string(nil), f.calls...)
}

func newTestService(t *testing.T, r Runner) *Service {
	t.Helper()
	dir := t.TempDir()
	s := New(nil, filepath.Join(dir, "jobs.json"), filepath.Join(dir, "jobs-history.json"), r)
	s.loc = time.UTC
	return s
}

func validJob(name string) Job {
	return Job{
		Name:    name,
		Enabled: true,
		Trigger: Trigger{Kind: "interval", EveryMinutes: 30},
		Action:  Action{Kind: "shell", Shell: &ShellAction{Command: "true"}},
	}
}

func upsert(t *testing.T, s *Service, j Job) Job {
	t.Helper()
	raw, _ := json.Marshal(j)
	res, err := s.Upsert(raw)
	if err != nil {
		t.Fatalf("upsert: %v", err)
	}
	return res.(Job)
}

func TestValidateRejectsTheSharpEdges(t *testing.T) {
	cases := []struct {
		name string
		mut  func(*Job)
		want string
	}{
		{"no name", func(j *Job) { j.Name = " " }, "needs a name"},
		{"bad trigger kind", func(j *Job) { j.Trigger = Trigger{Kind: "cron"} }, "unknown trigger"},
		{"interval zero", func(j *Job) { j.Trigger = Trigger{Kind: "interval"} }, "everyMinutes"},
		{"daily bad time", func(j *Job) { j.Trigger = Trigger{Kind: "daily", At: "25:99"} }, "HH:MM"},
		{"daily bad day", func(j *Job) { j.Trigger = Trigger{Kind: "daily", At: "09:00", Days: []int{7}} }, "out of range"},
		{"once bad time", func(j *Job) { j.Trigger = Trigger{Kind: "once", Once: "tomorrow"} }, "RFC3339"},
		{"spawn missing prompt", func(j *Job) {
			j.Action = Action{Kind: "spawn", Spawn: &SpawnAction{Cwd: "/x"}}
		}, "cwd and prompt"},
		{"call into jobs recurses", func(j *Job) {
			j.Action = Action{Kind: "call", Call: &CallAction{Method: "jobs.run"}}
		}, "may not target"},
		{"call across federation", func(j *Job) {
			j.Action = Action{Kind: "call", Call: &CallAction{Method: "hub:work/agents.spawn"}}
		}, "may not target"},
		{"shell empty", func(j *Job) { j.Action = Action{Kind: "shell", Shell: &ShellAction{}} }, "needs a command"},
	}
	for _, tc := range cases {
		j := validJob("x")
		tc.mut(&j)
		err := Validate(&j)
		if err == nil || !strings.Contains(err.Error(), tc.want) {
			t.Errorf("%s: want error containing %q, got %v", tc.name, tc.want, err)
		}
	}
	j := validJob("ok")
	if err := Validate(&j); err != nil {
		t.Fatalf("valid job refused: %v", err)
	}
}

func TestNextRunShapes(t *testing.T) {
	utc := time.UTC
	after := time.Date(2026, 8, 18, 10, 30, 0, 0, utc) // a Tuesday

	if at, ok := NextRun(Trigger{Kind: "interval", EveryMinutes: 45}, after, utc); !ok || !at.Equal(after.Add(45*time.Minute)) {
		t.Fatalf("interval: got %v ok=%v", at, ok)
	}
	// Daily at a time already past today → tomorrow (Wednesday allowed).
	if at, ok := NextRun(Trigger{Kind: "daily", At: "09:00"}, after, utc); !ok ||
		!at.Equal(time.Date(2026, 8, 19, 9, 0, 0, 0, utc)) {
		t.Fatalf("daily past-time: got %v ok=%v", at, ok)
	}
	// Daily at a later time today → today.
	if at, ok := NextRun(Trigger{Kind: "daily", At: "18:00"}, after, utc); !ok ||
		!at.Equal(time.Date(2026, 8, 18, 18, 0, 0, 0, utc)) {
		t.Fatalf("daily same-day: got %v ok=%v", at, ok)
	}
	// Weekday mask: Mon–Fri only, asked on Friday evening → Monday.
	fri := time.Date(2026, 8, 21, 20, 0, 0, 0, utc)
	if at, ok := NextRun(Trigger{Kind: "daily", At: "09:00", Days: []int{1, 2, 3, 4, 5}}, fri, utc); !ok ||
		!at.Equal(time.Date(2026, 8, 24, 9, 0, 0, 0, utc)) {
		t.Fatalf("weekday mask: got %v ok=%v", at, ok)
	}
	// Once: a past time still fires (missed-while-asleep must not mean never).
	past := Trigger{Kind: "once", Once: "2026-08-18T08:00:00Z"}
	if at, ok := NextRun(past, after, utc); !ok || !at.Before(after) {
		t.Fatalf("once-past: got %v ok=%v", at, ok)
	}
	if _, ok := NextRun(Trigger{Kind: "manual"}, after, utc); ok {
		t.Fatal("manual must not schedule")
	}
}

func TestUpsertListRemovePersistAcrossReload(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "jobs.json")
	hist := filepath.Join(dir, "jobs-history.json")
	s := New(nil, path, hist, &fakeRunner{})
	s.loc = time.UTC

	created := upsert(t, s, validJob("nightly build"))
	if created.ID == "" || created.CreatedAt == 0 {
		t.Fatalf("create must mint id + createdAt: %+v", created)
	}
	// Update keeps identity + creation time.
	created.Name = "renamed"
	updated := upsert(t, s, created)
	if updated.ID != created.ID || updated.CreatedAt != created.CreatedAt {
		t.Fatalf("update changed identity: %+v vs %+v", updated, created)
	}

	// A fresh service over the same files sees the job, with a next run.
	s2 := New(nil, path, hist, &fakeRunner{})
	s2.loc = time.UTC
	res, _ := s2.List(nil)
	views := res.(map[string]any)["jobs"].([]JobView)
	if len(views) != 1 || views[0].Name != "renamed" || views[0].NextRunAt == 0 {
		t.Fatalf("reload lost the job: %+v", views)
	}

	if _, err := s2.Remove(json.RawMessage(`{"id":"` + created.ID + `"}`)); err != nil {
		t.Fatalf("remove: %v", err)
	}
	s3 := New(nil, path, hist, &fakeRunner{})
	res3, _ := s3.List(nil)
	if got := res3.(map[string]any)["jobs"].([]JobView); len(got) != 0 {
		t.Fatalf("remove did not persist: %+v", got)
	}
}

// waitFor polls until the condition holds — execute() runs on a goroutine.
func waitFor(t *testing.T, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal("condition never held")
}

func lastRunOf(s *Service, id string) *Run {
	s.mu.Lock()
	defer s.mu.Unlock()
	if runs := s.history[id]; len(runs) > 0 {
		r := runs[0]
		return &r
	}
	return nil
}

func TestTickFiresDueIntervalJobAndReschedules(t *testing.T) {
	r := &fakeRunner{}
	s := newTestService(t, r)
	j := upsert(t, s, validJob("tick me"))

	// Not due yet: nothing runs.
	s.tick()
	if len(r.recorded()) != 0 {
		t.Fatalf("fired early: %v", r.recorded())
	}

	// Jump past the due time.
	s.now = func() time.Time { return time.Now().Add(31 * time.Minute) }
	s.tick()
	waitFor(t, func() bool { return lastRunOf(s, j.ID) != nil })
	if got := lastRunOf(s, j.ID); got.Status != "ok" {
		t.Fatalf("run: %+v", got)
	}
	// Rescheduled ~30m after the (shifted) now, i.e. ~61m out in real time.
	s.mu.Lock()
	next := s.nextAt[j.ID]
	s.mu.Unlock()
	if until := time.Until(next); until < 55*time.Minute || until > 65*time.Minute {
		t.Fatalf("bad reschedule: next in %v", until)
	}
}

func TestOnceJobDisablesItselfAtFireTime(t *testing.T) {
	r := &fakeRunner{}
	s := newTestService(t, r)
	j := validJob("one shot")
	j.Trigger = Trigger{Kind: "once", Once: time.Now().UTC().Add(-time.Minute).Format(time.RFC3339)}
	created := upsert(t, s, j)

	s.tick()
	waitFor(t, func() bool { return lastRunOf(s, created.ID) != nil })

	res, _ := s.List(nil)
	v := res.(map[string]any)["jobs"].([]JobView)[0]
	if v.Enabled || v.NextRunAt != 0 {
		t.Fatalf("once job must disable after firing: %+v", v)
	}
	// A later tick must not refire.
	before := len(r.recorded())
	s.tick()
	time.Sleep(20 * time.Millisecond)
	if len(r.recorded()) != before {
		t.Fatal("once job fired twice")
	}
}

func TestOverlapIsSkippedNotQueued(t *testing.T) {
	r := &fakeRunner{}
	s := newTestService(t, r)
	j := upsert(t, s, validJob("slow"))

	// Mark it running, then make it due: the tick must record a skip.
	s.mu.Lock()
	s.running[j.ID] = true
	s.mu.Unlock()
	s.now = func() time.Time { return time.Now().Add(31 * time.Minute) }
	s.tick()

	got := lastRunOf(s, j.ID)
	if got == nil || got.Status != "skipped" {
		t.Fatalf("want skipped run, got %+v", got)
	}
	if len(r.recorded()) != 0 {
		t.Fatalf("skipped run must not execute: %v", r.recorded())
	}
}

func TestSpawnActionSpawnsThenPrompts(t *testing.T) {
	r := &fakeRunner{}
	s := newTestService(t, r)
	j := validJob("triage")
	j.Action = Action{Kind: "spawn", Spawn: &SpawnAction{
		Cwd: "/home/u/repo", Prompt: "triage the overnight issues", Provider: "claude",
	}}
	created := upsert(t, s, j)

	s.execute(created) // synchronous: the direct path RunNow/tick go through
	calls := r.recorded()
	if len(calls) != 2 || calls[0] != "agents.spawn" || calls[1] != "agents.sendMessage" {
		t.Fatalf("calls: %v", calls)
	}
	spawnParams := r.params[0].(map[string]any)
	if spawnParams["cwd"] != "/home/u/repo" || spawnParams["label"] != "triage" {
		t.Fatalf("spawn params: %+v", spawnParams)
	}
	msgParams := r.params[1].(map[string]any)
	if msgParams["sessionId"] != "sess-1" || msgParams["text"] != "triage the overnight issues" {
		t.Fatalf("sendMessage params: %+v", msgParams)
	}
	if got := lastRunOf(s, created.ID); got.Status != "ok" || !strings.Contains(got.Detail, "sess-1") {
		t.Fatalf("run: %+v", got)
	}
}

func TestFailedRunRecordsError(t *testing.T) {
	r := &fakeRunner{fail: true}
	s := newTestService(t, r)
	j := validJob("broken")
	j.Action = Action{Kind: "call", Call: &CallAction{Method: "notifications.post"}}
	created := upsert(t, s, j)

	s.execute(created)
	got := lastRunOf(s, created.ID)
	if got == nil || got.Status != "error" || got.Detail == "" {
		t.Fatalf("want error run, got %+v", got)
	}
}

func TestRunNowRefusesWhileRunning(t *testing.T) {
	s := newTestService(t, &fakeRunner{})
	j := upsert(t, s, validJob("busy"))
	s.mu.Lock()
	s.running[j.ID] = true
	s.mu.Unlock()
	res, err := s.RunNow(json.RawMessage(`{"id":"` + j.ID + `"}`))
	if err != nil {
		t.Fatal(err)
	}
	if res.(map[string]any)["started"] != false {
		t.Fatalf("want started=false, got %+v", res)
	}
}

func TestHistoryIsCapped(t *testing.T) {
	s := newTestService(t, &fakeRunner{})
	s.mu.Lock()
	for i := 0; i < maxRunsPerJob+10; i++ {
		s.recordRunLocked(Run{JobID: "x", Status: "ok"})
	}
	n := len(s.history["x"])
	s.mu.Unlock()
	if n != maxRunsPerJob {
		t.Fatalf("history cap: %d", n)
	}
}
