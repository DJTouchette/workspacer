package quiescence

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"
)

var base = time.Date(2026, 8, 24, 12, 0, 0, 0, time.UTC)

// quietFleet is a reading with nothing wrong with it: one session ready for
// input, one client that stopped talking an hour ago, one job due tomorrow,
// one reachable peer. Every case below is this, with ONE thing changed.
func quietFleet() Inputs {
	return Inputs{
		Now: base,
		Sessions: []Session{
			{ID: "s1", Mode: "input"},
		},
		Clients: []Client{
			{Label: "view token", LastActive: base.Add(-time.Hour)},
		},
		Jobs: []Job{
			{ID: "j1", Name: "nightly review", ActionKind: "spawn", NextRun: base.Add(24 * time.Hour)},
		},
		Peers: []Peer{{Name: "laptop"}},
	}
}

func kinds(bs []Blocker) []string {
	out := make([]string, 0, len(bs))
	for _, b := range bs {
		out = append(out, b.Kind)
	}
	return out
}

func hasKind(bs []Blocker, kind string) bool {
	for _, b := range bs {
		if b.Kind == kind {
			return true
		}
	}
	return false
}

// ── one case per blocker ────────────────────────────────────────────────────

func TestEvaluate(t *testing.T) {
	cases := []struct {
		name string
		// mutate applies this case's ONE change to an otherwise-quiet fleet.
		mutate func(*Inputs)
		want   string // "" = expected to be calm
		// detailMust, when set, must appear in the blocker's Detail: the point
		// of a blocker is that an operator can act on it.
		detailMust string
	}{
		{
			name:   "a quiet fleet blocks on nothing",
			mutate: func(*Inputs) {},
		},
		{
			name:   "a session producing a turn",
			mutate: func(in *Inputs) { in.Sessions[0].Mode = "responding" },
			want:   KindSessionWorking,
		},
		{
			// THE ONE THIS WHOLE PREDICATE EXISTS FOR. `unknown` is the mode a
			// SPAWNING session sits in, and it is also where a terminal PTY
			// lives permanently, since nothing tracks a terminal's busy state.
			name:       "a session in the unknown mode is not idle",
			mutate:     func(in *Inputs) { in.Sessions[0].Mode = "unknown" },
			want:       KindSessionUnknown,
			detailMust: "TERMINAL",
		},
		{
			name:       "a mode from a newer daemon than this build knows",
			mutate:     func(in *Inputs) { in.Sessions[0].Mode = "compacting" },
			want:       KindSessionUnknown,
			detailMust: "compacting",
		},
		{
			name:   "a row with no state at all",
			mutate: func(in *Inputs) { in.Sessions[0] = Session{ID: "s1", Unreadable: "no mode"} },
			want:   KindSessionUnreadable,
		},
		{
			name:       "a background shell the mode deliberately does not carry",
			mutate:     func(in *Inputs) { in.Sessions[0].BackgroundTasks = 1 },
			want:       KindBackgroundTasks,
			detailMust: "dev server",
		},
		{
			name:   "a pending approval",
			mutate: func(in *Inputs) { in.Sessions[0].PendingApproval = true },
			want:   KindPendingApproval,
		},
		{
			name:   "a pending question",
			mutate: func(in *Inputs) { in.Sessions[0].PendingQuestion = true },
			want:   KindPendingQuestion,
		},
		{
			name:       "the fleet could not be read at all",
			mutate:     func(in *Inputs) { in.SessionsErr = errors.New("no provider for sessions.snapshots") },
			want:       KindFleetUnreadable,
			detailMust: "not an empty fleet",
		},
		{
			name:       "a client that did something a moment ago",
			mutate:     func(in *Inputs) { in.Clients[0].LastActive = base.Add(-30 * time.Second) },
			want:       KindClientActive,
			detailMust: "view token",
		},
		{
			// The distinction the whole client rule turns on: a phone holding a
			// socket open in a background tab is CONNECTED, and connected is
			// not in use. Only silence expires.
			name:   "a client connected but silent past the window is not active",
			mutate: func(in *Inputs) { in.Clients[0].LastActive = base.Add(-DefaultClientIdleWindow) },
		},
		{
			name:       "a scheduled job about to fire",
			mutate:     func(in *Inputs) { in.Jobs[0].NextRun = base.Add(4 * time.Minute) },
			want:       KindJobDueSoon,
			detailMust: "nightly review",
		},
		{
			name:   "a job running right now",
			mutate: func(in *Inputs) { in.Jobs[0].Running = true },
			want:   KindJobRunning,
		},
		{
			// Documented limitation, asserted so it cannot drift into being an
			// accident: the shell action is how this check is run, so a shell
			// poller that counted itself would block forever.
			name: "a shell job does not block, because the poller is one",
			mutate: func(in *Inputs) {
				in.Jobs[0] = Job{ID: "j2", Name: "sleep when quiet", ActionKind: "shell",
					NextRun: base.Add(time.Minute), Running: true}
			},
		},
		{
			name:       "a peer that could not be reached",
			mutate:     func(in *Inputs) { in.Peers[0].Err = "dial tcp: connection refused" },
			want:       KindPeerUnreachable,
			detailMust: "not a quiet one",
		},
		{
			name: "a peer's own session is read exactly like a local one",
			mutate: func(in *Inputs) {
				in.Sessions = append(in.Sessions, Session{ID: "p1", Peer: "laptop", Mode: "responding"})
			},
			want:       KindSessionWorking,
			detailMust: "hub:laptop/p1",
		},
		{
			name:   "an ended session blocks nothing",
			mutate: func(in *Inputs) { in.Sessions[0] = Session{ID: "s1", Mode: "stopped", Ended: true} },
		},
		{
			// The desktop provider speaks only the ambient vocabulary.
			name:   "a desktop-shape row that is working",
			mutate: func(in *Inputs) { in.Sessions[0] = Session{ID: "s1", Ambient: "streaming"} },
			want:   KindSessionWorking,
		},
		{
			name:   "a desktop-shape row with spawned work still running",
			mutate: func(in *Inputs) { in.Sessions[0] = Session{ID: "s1", Ambient: "background"} },
			want:   KindSessionWorking,
		},
		{
			name:   "a desktop-shape row at rest",
			mutate: func(in *Inputs) { in.Sessions[0] = Session{ID: "s1", Ambient: "idle"} },
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			in := quietFleet()
			tc.mutate(&in)
			got := Evaluate(in, Tunables{})
			if tc.want == "" {
				if len(got) != 0 {
					t.Fatalf("expected a calm reading, got blockers %v", kinds(got))
				}
				return
			}
			if !hasKind(got, tc.want) {
				t.Fatalf("expected a %q blocker, got %v", tc.want, kinds(got))
			}
			if tc.detailMust != "" {
				found := false
				for _, b := range got {
					if strings.Contains(b.Detail, tc.detailMust) || strings.Contains(b.ID, tc.detailMust) {
						found = true
					}
				}
				if !found {
					t.Errorf("no blocker names %q — the list is what an operator acts on: %+v", tc.detailMust, got)
				}
			}
		})
	}
}

// A mode arriving with the pending slot missing must still block on the mode.
func TestBlockedModesBlockWithoutTheSlot(t *testing.T) {
	for mode, want := range map[string]string{
		"approval": KindPendingApproval,
		"question": KindPendingQuestion,
	} {
		in := quietFleet()
		in.Sessions[0].Mode = mode
		if got := Evaluate(in, Tunables{}); !hasKind(got, want) {
			t.Errorf("mode %q gave %v, want a %q blocker", mode, kinds(got), want)
		}
	}
}

// ── the dwell ───────────────────────────────────────────────────────────────

// sample walks the monitor forward at a realistic sampler cadence, because
// observations further apart than MaxSampleGap deliberately do NOT count as
// continuous — see TestAnUnwatchedGapRestartsTheDwell.
func sample(m *Monitor, in Inputs, from, to time.Duration) Result {
	var res Result
	for at := from; at <= to; at += 30 * time.Second {
		in.Now = base.Add(at)
		res = m.Observe(in)
	}
	return res
}

func TestDwellNotYetElapsed(t *testing.T) {
	m := NewMonitor(Tunables{})
	in := quietFleet()
	res := m.Observe(in)
	if res.Quiescent {
		t.Fatal("quiescent on the FIRST calm reading — the dwell is what makes the absence of a liveness heartbeat tolerable, and skipping it is the whole risk")
	}
	if !hasKind(res.Blockers, KindDwell) {
		t.Fatalf("want a dwell blocker, got %v", kinds(res.Blockers))
	}

	res = sample(m, in, 30*time.Second, DefaultDwell-time.Minute)
	if res.Quiescent {
		t.Fatalf("quiescent a minute early (calm %ds of %ds)", res.CalmSeconds, res.DwellSeconds)
	}
	if !hasKind(res.Blockers, KindDwell) {
		t.Fatalf("want a dwell blocker, got %v", kinds(res.Blockers))
	}
}

func TestDwellElapsed(t *testing.T) {
	m := NewMonitor(Tunables{})
	in := quietFleet()
	res := sample(m, in, 0, DefaultDwell)
	if !res.Quiescent {
		t.Fatalf("not quiescent after the full dwell of calm: %v", kinds(res.Blockers))
	}
	if len(res.Blockers) != 0 {
		t.Errorf("quiescent with blockers: %v", res.Blockers)
	}
	if res.Since == nil || *res.Since != base.UnixMilli() {
		t.Errorf("since = %v, want the instant the calm BEGAN (%d)", res.Since, base.UnixMilli())
	}
}

// One blocked reading anywhere in the window restarts the clock. This is what
// "held continuously" means.
func TestABlockerRestartsTheDwell(t *testing.T) {
	m := NewMonitor(Tunables{})
	in := quietFleet()
	sample(m, in, 0, 10*time.Minute)

	busy := quietFleet()
	busy.Sessions[0].Mode = "responding"
	sample(m, busy, 10*time.Minute+30*time.Second, 11*time.Minute)

	if res := sample(m, in, 11*time.Minute+30*time.Second, 11*time.Minute+DefaultDwell-time.Minute); res.Quiescent {
		t.Fatal("a working session in the middle of the window did not restart the dwell")
	}
	if res := sample(m, in, 11*time.Minute+DefaultDwell-30*time.Second, 12*time.Minute+DefaultDwell); !res.Quiescent {
		t.Fatalf("not quiescent a full dwell after the fleet went calm again: %v", kinds(res.Blockers))
	}
}

// A gap nobody watched cannot be counted toward "held continuously".
func TestAnUnwatchedGapRestartsTheDwell(t *testing.T) {
	m := NewMonitor(Tunables{})
	in := quietFleet()
	m.Observe(in)
	// The hub was paused, suspended, or simply wedged for longer than the
	// sampler's own tolerance. Nothing was observed in between, so the fact
	// that both ends of the gap look calm proves nothing about the middle.
	in.Now = base.Add(DefaultDwell + DefaultMaxSampleGap + time.Minute)
	res := m.Observe(in)
	if res.Quiescent {
		t.Fatal("counted an unobserved gap toward the dwell — nothing was watching, so nothing is known about it")
	}
	if !hasKind(res.Blockers, KindDwell) {
		t.Fatalf("want the dwell to have restarted, got %v", kinds(res.Blockers))
	}
}

// A stale answer is refused rather than served: if the sampler stopped, the
// last thing it saw says nothing about now.
func TestLatestRefusesAStaleReading(t *testing.T) {
	m := NewMonitor(Tunables{})
	now := base
	m.SetClock(func() time.Time { return now })

	if res := m.Latest(); res.Quiescent || !hasKind(res.Blockers, KindStaleSample) {
		t.Fatalf("a monitor that has never sampled must refuse, got %+v", res)
	}
	in := quietFleet()
	sample(m, in, 0, DefaultDwell)
	now = base.Add(DefaultDwell)
	if res := m.Latest(); !res.Quiescent {
		t.Fatalf("a fresh reading should answer: %v", kinds(res.Blockers))
	}
	now = base.Add(DefaultDwell + DefaultMaxSampleGap + time.Second)
	res := m.Latest()
	if res.Quiescent {
		t.Fatal("served a stale answer as quiescent — the sampler had stopped")
	}
	if !hasKind(res.Blockers, KindStaleSample) {
		t.Fatalf("want a stale-sample blocker, got %v", kinds(res.Blockers))
	}
}

// An unreachable peer holds the answer at no for as long as it is unreachable,
// however calm everything local is.
func TestAnUnreachablePeerNeverGoesQuiescent(t *testing.T) {
	m := NewMonitor(Tunables{})
	in := quietFleet()
	in.Peers[0].Err = "dial tcp 100.64.0.2:7895: i/o timeout"
	for at := time.Duration(0); at <= DefaultDwell+2*time.Minute; at += 30 * time.Second {
		in.Now = base.Add(at)
		if res := m.Observe(in); res.Quiescent {
			t.Fatalf("quiescent at %s with a peer we could not reach", at)
		}
	}
	if res := m.Latest(); res.Quiescent {
		t.Fatal("Latest disagreed with Observe about an unreachable peer")
	}
}

// ── parsing ─────────────────────────────────────────────────────────────────

func TestParseSessionsReadsBothProviderShapes(t *testing.T) {
	// The headless brain's row: claudemon snake_case with the desktop overlay.
	// The desktop's row: camelCase only, no `mode` at all.
	raw := json.RawMessage(`[
		{"session_id":"brain-1","mode":"input","background_tasks":2,"ambientState":"background"},
		{"session_id":"brain-2","mode":"unknown"},
		{"session_id":"brain-3","mode":"stopped","status":"ended"},
		{"session_id":"brain-4","mode":"approval","pending":{"kind":"approval","tool":"Bash"}},
		{"sessionId":"desk-1","status":"active","ambientState":"streaming"},
		{"sessionId":"desk-2","status":"active","ambientState":"waiting_input",
		 "pendingQuestions":[{"question":"which?"}]},
		{"sessionId":"desk-3","status":"active","ambientState":"idle","backgroundTasks":1},
		{"cwd":"/nowhere"}
	]`)
	got, err := ParseSessions("", raw)
	if err != nil {
		t.Fatal(err)
	}
	if len(got) != 8 {
		t.Fatalf("parsed %d rows, want 8", len(got))
	}
	if got[0].Mode != "input" || got[0].BackgroundTasks != 2 {
		t.Errorf("brain row: %+v", got[0])
	}
	if got[1].Mode != "unknown" || got[1].Ended {
		t.Errorf("unknown row: %+v", got[1])
	}
	if !got[2].Ended {
		t.Errorf("stopped row not ended: %+v", got[2])
	}
	if !got[3].PendingApproval {
		t.Errorf("claudemon pending card not read: %+v", got[3])
	}
	if got[4].Ambient != "streaming" {
		t.Errorf("desktop row: %+v", got[4])
	}
	if !got[5].PendingQuestion {
		t.Errorf("desktop pendingQuestions not read: %+v", got[5])
	}
	if got[6].BackgroundTasks != 1 {
		t.Errorf("desktop backgroundTasks not read: %+v", got[6])
	}
	if got[7].Unreadable == "" {
		t.Errorf("a row with no id must be unreadable, not skipped: %+v", got[7])
	}

	// And the whole set must block: seven of the eight have something to say.
	in := Inputs{Now: base, Sessions: got}
	blockers := Evaluate(in, Tunables{})
	for _, want := range []string{
		KindBackgroundTasks, KindSessionUnknown, KindPendingApproval,
		KindSessionWorking, KindPendingQuestion, KindSessionUnreadable,
	} {
		if !hasKind(blockers, want) {
			t.Errorf("missing a %q blocker in %v", want, kinds(blockers))
		}
	}
}

func TestParseSessionsRefusesRatherThanReturningNothing(t *testing.T) {
	for _, raw := range []json.RawMessage{nil, json.RawMessage(`{"not":"an array"}`), json.RawMessage(`nope`)} {
		if _, err := ParseSessions("", raw); err == nil {
			t.Errorf("parsed %q without error — an unreadable answer must not read as an empty fleet", raw)
		}
	}
	// A genuinely empty fleet is fine and is NOT an error.
	got, err := ParseSessions("", json.RawMessage(`[]`))
	if err != nil || len(got) != 0 {
		t.Errorf("empty fleet: %v %v", got, err)
	}
}

func TestPeerRowsAreNamedByPeer(t *testing.T) {
	got, err := ParseSessions("laptop", json.RawMessage(`[{"session_id":"s9","mode":"responding"}]`))
	if err != nil {
		t.Fatal(err)
	}
	if got[0].Ref() != "hub:laptop/s9" {
		t.Errorf("Ref() = %q, want the qualified form the bus already uses", got[0].Ref())
	}
}

// Two identical readings must render identically: a list that shuffles between
// polls reads like state changing when nothing has.
func TestBlockerOrderIsStable(t *testing.T) {
	m := NewMonitor(Tunables{})
	in := quietFleet()
	in.Sessions = []Session{
		{ID: "z", Mode: "responding"},
		{ID: "a", Mode: "unknown"},
		{ID: "m", Mode: "input", BackgroundTasks: 1},
	}
	first := m.Observe(in)
	in.Now = base.Add(time.Minute)
	second := m.Observe(in)
	if len(first.Blockers) != len(second.Blockers) {
		t.Fatalf("%d vs %d blockers", len(first.Blockers), len(second.Blockers))
	}
	for i := range first.Blockers {
		if first.Blockers[i] != second.Blockers[i] {
			t.Fatalf("blocker %d moved: %+v vs %+v", i, first.Blockers[i], second.Blockers[i])
		}
	}
}
