package main

// The worker-finished wake, headless: the transition that produces it, the two
// re-verifications that stop it lying, the coalescing that stops it storming,
// and the catch-up backstop behind all of it.
//
// The thing under test is a WATCHER over snapshots, so each case drives it the
// way production does — enrich a claudemon row into the store, let onChange fire
// observe() — and then closes the coalesce window explicitly instead of
// sleeping through 1.5 real seconds.

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

// ── harness ─────────────────────────────────────────────────────────────────

// wakeDaemon is a fake claudemon that serves per-session conversations and
// records the messages injected into each session.
type wakeDaemon struct {
	mu sync.Mutex
	// conv is the JSON body served for GET /sessions/<id>/conversation. A
	// session with no entry gets an empty item list, which reads as a BOOT IDLE
	// (no user turn yet) — the same thing production sees for a worker whose
	// task message has not landed.
	conv map[string]string
	// refuse names sessions whose POST /message answers 409 (the daemon
	// refusing input for an ended session).
	refuse map[string]bool
	sent   []sentMessage
}

type sentMessage struct{ to, text string }

func newWakeDaemon() *wakeDaemon {
	return &wakeDaemon{conv: map[string]string{}, refuse: map[string]bool{}}
}

// dispatched is the conversation of a worker that WAS given its task and
// replied — the ordinary case.
func dispatched(reply string) string {
	items := []map[string]any{{"kind": "user_message", "text": "SHIP TASK — do the thing"}}
	if reply != "" {
		items = append(items, map[string]any{"kind": "assistant_text", "text": reply})
	}
	body, _ := json.Marshal(map[string]any{"seq": len(items), "items": items})
	return string(body)
}

func (d *wakeDaemon) setConv(id, body string) {
	d.mu.Lock()
	d.conv[id] = body
	d.mu.Unlock()
}

func (d *wakeDaemon) server() *httptest.Server {
	return httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		parts := strings.Split(strings.Trim(r.URL.Path, "/"), "/")
		if len(parts) == 3 && parts[0] == "sessions" && parts[2] == "conversation" {
			d.mu.Lock()
			body := d.conv[parts[1]]
			d.mu.Unlock()
			if body == "" {
				body = `{"seq":0,"items":[]}`
			}
			w.Write([]byte(body))
			return
		}
		if len(parts) == 3 && parts[0] == "sessions" && parts[2] == "message" {
			var in struct {
				Text string `json:"text"`
			}
			_ = json.NewDecoder(r.Body).Decode(&in)
			d.mu.Lock()
			refused := d.refuse[parts[1]]
			if !refused {
				d.sent = append(d.sent, sentMessage{to: parts[1], text: in.Text})
			}
			d.mu.Unlock()
			if refused {
				w.WriteHeader(http.StatusConflict)
				w.Write([]byte(`{"ok":false}`))
				return
			}
			w.Write([]byte(`{"ok":true}`))
			return
		}
		w.Write([]byte(`{"ok":true}`))
	}))
}

func (d *wakeDaemon) to(id string) []string {
	d.mu.Lock()
	defer d.mu.Unlock()
	var out []string
	for _, m := range d.sent {
		if m.to == id {
			out = append(out, m.text)
		}
	}
	return out
}

// wakeRig is a full-scope registry with a live store, wired to a finish watcher
// whose coalesce timers this test drives by hand.
type wakeRig struct {
	t     *testing.T
	reg   *registry
	store *sessionStore
	meta  *metaStore
	fin   *finishWatcher
	d     *wakeDaemon

	mu      sync.Mutex
	pending []func()
}

func newWakeRig(t *testing.T) *wakeRig {
	t.Helper()
	d := newWakeDaemon()
	srv := d.server()
	t.Cleanup(srv.Close)

	reg := newRegistry(newClaudemonClient(srv.URL))
	meta := newMetaStore()
	reg.meta = meta
	store := newSessionStore()
	store.enrich = func(snap json.RawMessage) json.RawMessage { return enrichAndCompat(snap, meta) }
	reg.store = store

	rig := &wakeRig{t: t, reg: reg, store: store, meta: meta, d: d}
	fin := newFinishWatcher(reg)
	fin.after = func(_ time.Duration, fn func()) *time.Timer {
		rig.mu.Lock()
		rig.pending = append(rig.pending, fn)
		rig.mu.Unlock()
		// A timer that will not fire on its own: the test closes the window.
		t := time.NewTimer(time.Hour)
		t.Stop()
		return t
	}
	rig.fin = fin
	reg.fin = fin
	store.onSeed = fin.prime
	store.onChange = func(_ string, snap json.RawMessage) { fin.observe(context.Background(), snap) }
	return rig
}

// spawnMetaFor records a session's label/parent/manager flag, exactly as the
// spawn handler does.
func (r *wakeRig) spawnMetaFor(id string, m spawnMeta) { r.meta.set(id, m) }

// update pushes one claudemon row through the store, which is what fires the
// watcher — the production path, not a direct call into it.
func (r *wakeRig) update(id, cwd, mode string) {
	r.store.set(id, json.RawMessage(`{"session_id":"`+id+`","cwd":"`+cwd+`","mode":"`+mode+`"}`))
}

// closeWindows runs every coalesce timer that is open, which is what the 1.5s
// wall clock does in production.
func (r *wakeRig) closeWindows() {
	r.mu.Lock()
	fns := r.pending
	r.pending = nil
	r.mu.Unlock()
	for _, fn := range fns {
		fn()
	}
}

func (r *wakeRig) openWindows() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.pending)
}

// manager + worker is the shape every case below starts from.
func (r *wakeRig) fleet() {
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{Label: "rust worker", ParentSessionID: "mgr"})
	r.update("mgr", "/work", "input")
	r.update("w1", "/work/proj", "responding")
	r.d.setConv("w1", dispatched("All 42 tests pass. Merged as abc1234."))
}

// ── the wake itself ─────────────────────────────────────────────────────────

// THE WHOLE POINT: a worker going working→idle wakes the session that
// dispatched it, and nobody else. Without this a headless manager dispatches
// and then never hears back — silently, forever.
func TestAWorkerFinishingWakesItsHeadlessManager(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.spawnMetaFor("other-mgr", spawnMeta{Label: "Other", IsWakeTarget: true})
	r.update("other-mgr", "/elsewhere", "input")

	r.update("w1", "/work/proj", "input") // streaming → idle: the finish
	if r.openWindows() != 1 {
		t.Fatalf("the finish opened %d coalesce windows, want 1", r.openWindows())
	}
	r.closeWindows()

	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("the manager received %d wakes, want 1", len(wakes))
	}
	if !strings.HasPrefix(wakes[0], fleetWorkerFinishedHeader) {
		t.Errorf("wake does not open with the worker-finished header (every client's card parser keys off it):\n%s", wakes[0])
	}
	if !strings.Contains(wakes[0], "- rust worker (session:w1, cwd /work/proj) — last reply: All 42 tests pass. Merged as abc1234.") {
		t.Errorf("the bullet lost the worker's label, cwd or reply:\n%s", wakes[0])
	}
	if !strings.HasSuffix(wakes[0], fleetWorkerFinishedTail) {
		t.Errorf("wake lost its instruction tail — the manager was trained on it:\n%s", wakes[0])
	}
	// CONTAINMENT: a finish is parent-keyed, unlike a blocked broadcast. Waking
	// unrelated managers about someone else's dispatch is noise they pay a full
	// turn of context for.
	if n := len(r.d.to("other-mgr")); n != 0 {
		t.Errorf("a manager that dispatched nothing received %d wakes", n)
	}
}

// A parent that is not a MANAGER is not a wake target. `manager: true` at spawn
// is the only thing that sets isWakeTarget, and without it the finish is
// dropped — the same rule the desktop enforces.
func TestAFinishIsDroppedWhenTheParentIsNotAManager(t *testing.T) {
	r := newWakeRig(t)
	r.spawnMetaFor("plain", spawnMeta{Label: "not a manager"})
	r.spawnMetaFor("w1", spawnMeta{ParentSessionID: "plain"})
	r.update("plain", "/work", "input")
	r.update("w1", "/work/p", "responding")
	r.d.setConv("w1", dispatched("done"))

	r.update("w1", "/work/p", "input")
	r.closeWindows()
	if n := len(r.d.to("plain")); n != 0 {
		t.Errorf("a non-manager parent received %d wakes", n)
	}
}

// A worker with no parent at all wakes nobody and arms nothing.
func TestAnUnparentedFinishArmsNothing(t *testing.T) {
	r := newWakeRig(t)
	r.update("solo", "/work", "responding")
	r.d.setConv("solo", dispatched("done"))
	r.update("solo", "/work", "input")
	if r.openWindows() != 0 {
		t.Errorf("an unparented finish opened %d coalesce windows", r.openWindows())
	}
}

// ── the transition rule ─────────────────────────────────────────────────────

// A block CLEARING is not a dispatch coming home. Treating waiting_approval →
// idle as a finish is how a manager gets woken twice for one piece of work.
func TestOnlyAWorkingToIdleEdgeIsAFinish(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.update("w1", "/work/proj", "approval") // streaming → waiting_approval
	r.update("w1", "/work/proj", "input")    // waiting_approval → idle
	if r.openWindows() != 0 {
		t.Fatalf("a block clearing was treated as a finish (%d windows opened)", r.openWindows())
	}
	// …and the real finish, once it comes, still fires.
	r.update("w1", "/work/proj", "responding")
	r.update("w1", "/work/proj", "input")
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("the real finish produced %d wakes, want 1", n)
	}
}

// A FIRST SIGHTING is not a transition: a session's first snapshot has no
// before-state, so an agent that happens to appear idle must not read as one
// that just finished.
func TestAFirstSightingIsNotAFinish(t *testing.T) {
	r := newWakeRig(t)
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{ParentSessionID: "mgr"})
	r.update("mgr", "/work", "input")
	r.d.setConv("w1", dispatched("done"))
	r.update("w1", "/work/p", "input") // the very first row we ever see for w1
	if r.openWindows() != 0 {
		t.Errorf("a first sighting was treated as a finish (%d windows)", r.openWindows())
	}
}

// The SEEDED half of the same rule, and the reason store.onSeed exists: a
// worker that was already mid-turn when the brain started must still have its
// finish seen. Without priming, prevAmbient is empty at boot and that finish
// looks like a first sighting — the manager is dark until the 3-minute catch-up.
func TestPrimingFromTheSeedSeesTheFirstFinishAfterBoot(t *testing.T) {
	r := newWakeRig(t)
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{Label: "worker", ParentSessionID: "mgr"})
	r.d.setConv("w1", dispatched("done"))
	r.store.seed(map[string]json.RawMessage{
		"mgr": json.RawMessage(`{"session_id":"mgr","cwd":"/work","mode":"input"}`),
		"w1":  json.RawMessage(`{"session_id":"w1","cwd":"/work/p","mode":"responding"}`),
	})

	r.update("w1", "/work/p", "input")
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 1 {
		t.Fatalf("the first finish after boot produced %d wakes, want 1 — the seed did not prime the watcher", n)
	}
}

// ── coalescing ──────────────────────────────────────────────────────────────

// Five workers landing together is ONE manager turn, not five. Each wake costs
// the manager a full turn of context, and each one interrupts the last.
func TestFinishesForOneManagerCoalesceIntoOneWake(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.spawnMetaFor("w2", spawnMeta{Label: "go worker", ParentSessionID: "mgr"})
	r.update("w2", "/work/other", "responding")
	r.d.setConv("w2", dispatched("Also done."))

	r.update("w1", "/work/proj", "input")
	r.update("w2", "/work/other", "input")
	if r.openWindows() != 1 {
		t.Fatalf("two finishes for one manager opened %d windows, want 1", r.openWindows())
	}
	r.closeWindows()

	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("got %d wakes, want 1 carrying both workers", len(wakes))
	}
	if !strings.Contains(wakes[0], "session:w1") || !strings.Contains(wakes[0], "session:w2") {
		t.Errorf("the coalesced wake lost an entry:\n%s", wakes[0])
	}
}

// Different managers do NOT share a window — coalescing is per parent, and one
// manager's burst must not delay or absorb another's report.
func TestFinishesForDifferentManagersDoNotShareAWindow(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.spawnMetaFor("mgr2", spawnMeta{Label: "Manager Two", IsWakeTarget: true})
	r.spawnMetaFor("w2", spawnMeta{Label: "worker two", ParentSessionID: "mgr2"})
	r.update("mgr2", "/other", "input")
	r.update("w2", "/other/p", "responding")
	r.d.setConv("w2", dispatched("done two"))

	r.update("w1", "/work/proj", "input")
	r.update("w2", "/other/p", "input")
	if r.openWindows() != 2 {
		t.Fatalf("two managers' finishes opened %d windows, want 2", r.openWindows())
	}
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("mgr got %d wakes, want 1", n)
	}
	if n := len(r.d.to("mgr2")); n != 1 {
		t.Errorf("mgr2 got %d wakes, want 1", n)
	}
}

// ── re-verification at SEND time ────────────────────────────────────────────

// An idle BLIP mid-stream is not a finish. The worker is streaming again by the
// time the window closes, and reporting it would present a half-done result to
// the manager as final.
func TestAWorkerThatResumedWorkingInsideTheWindowIsNotReported(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.update("w1", "/work/proj", "input")      // schedules the wake
	r.update("w1", "/work/proj", "responding") // …and it was a blip
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("an idle blip was reported as a finish (%d wakes) — the manager would read a half-done result as final", n)
	}
}

// The PARENT is re-checked too. It can end, be closed, or be reparented out of
// manager-hood inside the 1.5s window, and a wake to a session that is no longer
// a manager is a wake nobody reads.
func TestAManagerThatEndedInsideTheWindowIsNotWoken(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.update("w1", "/work/proj", "input")
	r.update("mgr", "/work", "stopped") // the manager died inside the window
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a wake was composed and sent to an ended manager (%d) — the trigger-time check is not enough on its own", n)
	}
}

// The final assistant message can land AFTER the Stop edge (claudemon keeps
// tailing briefly), so the reply is re-read at delivery rather than captured
// when the wake was scheduled.
func TestTheReplyIsReReadAtDeliveryNotAtTrigger(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.d.setConv("w1", dispatched("")) // nothing written yet at the Stop edge
	r.update("w1", "/work/proj", "input")
	r.d.setConv("w1", dispatched("The report landed late.")) // …flushed after
	r.closeWindows()

	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("got %d wakes, want 1", len(wakes))
	}
	if !strings.Contains(wakes[0], "last reply: The report landed late.") {
		t.Errorf("the wake carried the schedule-time reply, not the live one:\n%s", wakes[0])
	}
}

// A session that has received NO user turn was never given its task: its idle
// is a BOOT idle, and waking the manager about it hands it an empty session.
func TestABootIdleIsNotAFinish(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.d.setConv("w1", `{"seq":1,"items":[{"kind":"assistant_text","text":"booting"}]}`)
	r.update("w1", "/work/proj", "input")
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a boot idle woke the manager (%d wakes)", n)
	}
}

// …but an unreadable conversation fails OPEN. A manager dark forever is a worse
// outcome than one wasted turn reading a session with no excerpt.
func TestAnUnreachableConversationStillDeliversTheWake(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	// Point the client at a dead address so the conversation fetch errors.
	r.reg.cm = newClaudemonClient("http://127.0.0.1:1")
	r.update("w1", "/work/proj", "input")
	// The delivery must still reach the real daemon.
	r.closeWindows()
	if r.openWindows() != 0 {
		t.Fatal("window did not close")
	}
}

// ── the duplicate-wake dedup ────────────────────────────────────────────────

// A repeat working→idle edge with the identical reply and status is noise — a
// flapping block, a re-derived Stop — and must not re-wake the parent. A
// genuinely different reply always changes the signature and still fires.
func TestARepeatEdgeWithNothingNewDoesNotReWakeTheManager(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.update("w1", "/work/proj", "input")
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 1 {
		t.Fatalf("first finish produced %d wakes, want 1", n)
	}

	r.update("w1", "/work/proj", "responding")
	r.update("w1", "/work/proj", "input") // same reply, same status
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("an identical repeat edge re-woke the manager (%d wakes total)", n)
	}

	// A second instruction producing a NEW reply is the wanted case.
	r.d.setConv("w1", dispatched("Second pass done."))
	r.update("w1", "/work/proj", "responding")
	r.update("w1", "/work/proj", "input")
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 2 {
		t.Errorf("a genuinely new reply produced %d wakes total, want 2", n)
	}
}

// A send that FAILS must not book the dedup signature. Booking it would turn a
// lost wake into a permanently silenced one: the next identical edge would
// dedup against a report nobody ever received.
func TestAFailedSendDoesNotSilenceTheNextIdenticalEdge(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.d.mu.Lock()
	r.d.refuse["mgr"] = true
	r.d.mu.Unlock()

	r.update("w1", "/work/proj", "input")
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 0 {
		t.Fatalf("the refused send was recorded as delivered (%d)", n)
	}

	r.d.mu.Lock()
	r.d.refuse["mgr"] = false
	r.d.mu.Unlock()
	r.update("w1", "/work/proj", "responding")
	r.update("w1", "/work/proj", "input") // the identical edge, retried
	r.closeWindows()
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("the retry was deduped against a wake that never landed (%d wakes)", n)
	}
}

// ── what the bullet says ────────────────────────────────────────────────────

// A worker that DIED must not be reported under a header containing the word
// "finished" — that is the exact sentence a manager reads as a landed outcome.
func TestAnAllFailedWakeUsesTheHonestHeaderAndCarriesTheFailedNote(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.d.setConv("w1", dispatched(agentErrorMarker+"Credit balance is too low to access the Anthropic API."))
	r.update("w1", "/work/proj", "input")
	r.closeWindows()

	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("got %d wakes, want 1", len(wakes))
	}
	if !strings.HasPrefix(wakes[0], fleetWorkerFailedHeader) {
		t.Errorf("a wake whose every worker died opened with %q:\n%s", fleetWorkerFinishedHeader, wakes[0])
	}
	if !strings.Contains(wakes[0], "— FAILED: Credit balance is too low to access the Anthropic API.") {
		t.Errorf("the bullet did not name the failure:\n%s", wakes[0])
	}
	if !strings.Contains(wakes[0], fleetFailedNote) {
		t.Errorf("the wake lost the FAILED note, which is what tells the manager not to book a crash as work landed:\n%s", wakes[0])
	}
}

// A session that ENDED reads as stopped/killed, on a separate axis from FAILED:
// the session going away is not the same event as the API refusing.
func TestAnEndedWorkerIsReportedStoppedWithItsNote(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.update("w1", "/work/proj", "stopped")
	r.closeWindows()

	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("got %d wakes, want 1", len(wakes))
	}
	if !strings.Contains(wakes[0], "(session:w1, cwd /work/proj) — stopped/killed — last reply:") {
		t.Errorf("the bullet did not mark the ended session stopped/killed:\n%s", wakes[0])
	}
	if !strings.Contains(wakes[0], fleetStoppedNote) {
		t.Errorf("the wake lost the stopped note:\n%s", wakes[0])
	}
	// It ENDED; it did not report an error. The two axes must stay separate.
	if strings.Contains(wakes[0], "FAILED") {
		t.Errorf("a stopped session was also badged FAILED:\n%s", wakes[0])
	}
}

// The COMPLETE final message rides the wake when the bullet excerpt is lossy —
// the whole reason a manager never has to fetch a conversation to read a report.
func TestALongReplyCarriesItsCompleteFinalMessage(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	long := strings.Repeat("a very long report. ", 60) // > the 400-char excerpt
	r.d.setConv("w1", dispatched(long))
	r.update("w1", "/work/proj", "input")
	r.closeWindows()

	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("got %d wakes, want 1", len(wakes))
	}
	if !strings.Contains(wakes[0], "…") {
		t.Errorf("the bullet excerpt was not truncated:\n%s", wakes[0])
	}
	if !strings.Contains(wakes[0], "Full final message — rust worker (session:w1):") {
		t.Errorf("the wake lost the full-reply block:\n%s", wakes[0])
	}
	if !strings.Contains(wakes[0], strings.TrimSpace(long)) {
		t.Error("the full-reply block did not carry the complete message")
	}
}

// A reply that already fits carries NO full-reply block — the bullet already IS
// the whole reply, and repeating it doubles the manager's reading for nothing.
func TestAShortReplyCarriesNoFullReplyBlock(t *testing.T) {
	r := newWakeRig(t)
	r.fleet()
	r.update("w1", "/work/proj", "input")
	r.closeWindows()
	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("got %d wakes, want 1", len(wakes))
	}
	if strings.Contains(wakes[0], "Full final message — ") {
		t.Errorf("a short reply was repeated in a full-reply block:\n%s", wakes[0])
	}
}

// ── the catch-up backstop ───────────────────────────────────────────────────

// The dark-manager recovery: a finish whose live wake never landed is re-sent
// once it is old enough that a normal wake would have arrived.
func TestTheBackstopCatchesAFinishWhoseWakeNeverLanded(t *testing.T) {
	r := newWakeRig(t)
	now := time.Now()
	managerActed := now.Add(-30 * time.Minute)
	childFinished := now.Add(-10 * time.Minute)
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{Label: "rust worker", ParentSessionID: "mgr"})
	r.store.set("mgr", json.RawMessage(atTime("mgr", "/work", "input", managerActed)))
	r.store.set("w1", json.RawMessage(atTime("w1", "/work/proj", "input", childFinished)))
	r.d.setConv("w1", dispatched("It landed, you just never heard."))

	r.fin.sweepMissedFinishes(context.Background(), now)
	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("the backstop produced %d wakes, want 1", len(wakes))
	}
	if !strings.HasPrefix(wakes[0], fleetCatchUpHeader) {
		t.Errorf("the catch-up did not use its own header — the manager must be told it may already have seen this:\n%s", wakes[0])
	}
	if !strings.Contains(wakes[0], "rust worker (session:w1, cwd /work/proj)") {
		t.Errorf("the catch-up bullet is wrong:\n%s", wakes[0])
	}
}

// The dedup is implicit and exact: a manager that ACTED after the child
// finished has already dealt with it, so there is nothing to catch up.
func TestTheBackstopSaysNothingWhenTheManagerAlreadyActed(t *testing.T) {
	r := newWakeRig(t)
	now := time.Now()
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{ParentSessionID: "mgr"})
	r.store.set("mgr", json.RawMessage(atTime("mgr", "/work", "input", now.Add(-5*time.Minute))))
	r.store.set("w1", json.RawMessage(atTime("w1", "/work/p", "input", now.Add(-10*time.Minute))))
	r.d.setConv("w1", dispatched("done"))

	r.fin.sweepMissedFinishes(context.Background(), now)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("the backstop re-nudged a manager that had already acted (%d wakes)", n)
	}
}

// A finish inside the grace window is not "missed" — it is mid-flight, and the
// normal path has not finished with it yet.
func TestTheBackstopRespectsTheGraceWindow(t *testing.T) {
	r := newWakeRig(t)
	now := time.Now()
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{ParentSessionID: "mgr"})
	r.store.set("mgr", json.RawMessage(atTime("mgr", "/work", "input", now.Add(-30*time.Minute))))
	r.store.set("w1", json.RawMessage(atTime("w1", "/work/p", "input", now.Add(-30*time.Second))))
	r.d.setConv("w1", dispatched("done"))

	r.fin.sweepMissedFinishes(context.Background(), now)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("the backstop fired inside the grace window (%d wakes)", n)
	}
}

// A manager that is BUSY is not dark — it is working, and interrupting it with
// a catch-up is the polling the doctrine exists to avoid.
func TestTheBackstopSkipsABusyManager(t *testing.T) {
	r := newWakeRig(t)
	now := time.Now()
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{ParentSessionID: "mgr"})
	r.store.set("mgr", json.RawMessage(atTime("mgr", "/work", "responding", now.Add(-30*time.Minute))))
	r.store.set("w1", json.RawMessage(atTime("w1", "/work/p", "input", now.Add(-10*time.Minute))))
	r.d.setConv("w1", dispatched("done"))

	r.fin.sweepMissedFinishes(context.Background(), now)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("the backstop interrupted a working manager (%d wakes)", n)
	}
}

// atTime is a claudemon row whose updated_at (→ lastActivity) is fixed, which is
// the field the backstop's whole decision rests on.
func atTime(id, cwd, mode string, t time.Time) string {
	return `{"session_id":"` + id + `","cwd":"` + cwd + `","mode":"` + mode +
		`","updated_at":"` + t.UTC().Format(time.RFC3339) + `"}`
}
