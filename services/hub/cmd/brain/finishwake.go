package main

// THE WORKER-FINISHED WAKE, HEADLESS — the thing that makes the Fleet Manager
// model work at all when there is no desktop.
//
// TWIN: apps/desktop/src/main/services/supervisorNudge.ts (onFinished →
// sendFinished, sweepMissedFinishes) plus the transition site that drives it,
// claudeSessionStore.nudgeParentOnFinish. The desktop keeps that machinery
// exactly as it is; this is the brain gaining its own equivalent, not a shared
// rewrite of both.
//
// WHY IT HAD TO EXIST. The manager's own doctrine (renderer/src/lib/
// fleetManager.ts, MANAGER_PREAMBLE rule 2) is NEVER POLL: it dispatches, ends
// its turn, and waits to be told. The desktop tells it. Under `workspacer serve`
// nothing did — the brain ran no equivalent under any name, so a headless
// manager dispatched workers and then sat idle forever while they finished into
// the void. The failure is SILENT on both sides: the worker has no idea whether
// its report was routed, and the manager cannot distinguish "nothing has
// finished yet" from "I am deaf".
//
// WHAT IS AND IS NOT PORTED HERE:
//
//   - worker-finished ✅ — the wake itself, with both of the parts that exist for
//     a reason and are easy to drop (see below).
//   - catch-up ✅ — the 2-minute backstop sweep. It is not decoration: this
//     watcher only sees a transition it was RUNNING for, so the brain's own
//     restart, a claudemon /events reconnect that drops an edge, or a wake whose
//     delivery threw all leave a manager dark with no other recovery.
//   - blocked ❌ — the approval/question broadcast. Deliberately out: it is the
//     one wake that is not parent-keyed (every live wake target receives it), it
//     needs its own 20s survive-this-long debounce with a matching clear edge,
//     and it answers a different question. A headless manager therefore still
//     does not learn that one of its workers is stuck on an approval; it learns
//     only when that worker eventually finishes.
//   - the structured-result half of a finish ❌ — `resultSchema` is declined by
//     the headless spawn (parity_test.go's spawnParamsDeclined), so there is no
//     contract to validate against. The prose report is delivered in full.
//
// THE TWO PARTS THAT ARE EASY TO DROP, and why neither is:
//
//  1. COALESCING (fleetFinishCoalesce). A manager that dispatched five workers
//     into one build gets ONE wake when they land together, not five turns'
//     worth. Without it a burst of finishes is a burst of manager turns, each
//     one costing context and each one interrupting the last.
//  2. RE-VERIFICATION AT SEND TIME, of both ends. The edge that scheduled a wake
//     can lie: a worker that blipped idle mid-stream is working again by the time
//     the window closes (reporting it would present a half-done result as final),
//     and the parent can have ended, been closed, or stopped being a manager in
//     the same 1.5 seconds. Checking either only at trigger time is exactly where
//     duplicate wakes and wakes to dead sessions come from.
//
// WHERE IT DIFFERS FROM THE DESKTOP, honestly:
//
//   - The desktop holds a live conversation per session and re-reads the reply
//     from memory. The brain's rows are SPARSE by design (conversation.go: folding
//     a transcript into every state tick would ship whole transcripts per state
//     change), so the reply and the has-it-received-a-task check are ONE
//     claudemon fetch per worker, made at delivery — the moment the desktop
//     re-reads from memory. A fetch that fails fails OPEN on the task check, the
//     same way an untracked conversation does on the desktop.
//   - The brain never sees an ambient state of "thinking" (ambientForMode does
//     not produce one), so "was working" here is streaming/background.

import (
	"context"
	"encoding/json"
	"log"
	"strings"
	"sync"
	"time"
)

const (
	// fleetFinishCoalesce is how long finishes for ONE parent are gathered
	// before a wake goes out. TWIN: COALESCE_MS.
	fleetFinishCoalesce = 1500 * time.Millisecond

	// fleetWakeBackstopInterval is how often the missed-wake sweep runs, and
	// fleetMissedWakeGrace is how old a finish must be before a still-idle
	// manager counts as having MISSED it — long enough that the normal path
	// (coalesce + deliver + the manager's own response) has had every chance to
	// land. TWIN: WAKE_BACKSTOP_MS and MISSED_WAKE_GRACE_MS.
	fleetWakeBackstopInterval = 2 * time.Minute
	fleetMissedWakeGrace      = 3 * time.Minute
)

// finishWatcher turns claudemon session-state transitions into worker-finished
// wakes. One per brain, in full scope only.
type finishWatcher struct {
	reg *registry

	// coalesce and after are injectable so a test can drive the window
	// deterministically instead of sleeping through it. after defaults to
	// time.AfterFunc.
	coalesce time.Duration
	after    func(time.Duration, func()) *time.Timer

	mu sync.Mutex
	// prevAmbient is the last ambient state we saw per session — the other half
	// of a transition, which a snapshot stream does not carry.
	prevAmbient map[string]string
	// pending is the open coalesce window per PARENT: the worker ids that
	// finished into it, in arrival order so a multi-entry wake is deterministic.
	pending map[string][]string
	timers  map[string]*time.Timer
	// lastReported is the signature (reply + stopped + failed) of the last wake
	// actually DELIVERED for a worker. A working→idle edge is a genuine finish
	// only the first time; a repeat edge with an identical reply and status is
	// noise, not a new report. TWIN: lastReportedReply, and
	// apps/desktop/PER_TURN_WAKE_FINDING.md for the observed duplicate.
	lastReported map[string]string
}

func newFinishWatcher(reg *registry) *finishWatcher {
	return &finishWatcher{
		reg:          reg,
		coalesce:     fleetFinishCoalesce,
		after:        time.AfterFunc,
		prevAmbient:  map[string]string{},
		pending:      map[string][]string{},
		timers:       map[string]*time.Timer{},
		lastReported: map[string]string{},
	}
}

// wasWorking reports whether an ambient state is one a finish can transition
// OUT of. TWIN: the `thinking | streaming | background` test in
// nudgeParentOnFinish — minus 'thinking', which ambientForMode never produces.
//
// waiting_approval and waiting_input are deliberately absent: a block clearing
// is not a dispatch coming home, and treating it as one is how a manager gets
// woken twice for one piece of work.
func wasWorking(ambient string) bool {
	return ambient == "streaming" || ambient == "background" || ambient == "thinking"
}

// prime records the ambient state of every session already present when the
// store is seeded, WITHOUT waking anyone.
//
// Without it the first transition after boot is invisible: `prevAmbient` would
// be empty, so a worker that was mid-turn when the brain started and finished a
// second later would be recorded as "first sighting: idle" and its manager would
// never hear. The catch-up sweep would recover that eventually, three minutes
// later; priming makes it not happen.
func (w *finishWatcher) prime(snaps map[string]json.RawMessage) {
	w.mu.Lock()
	defer w.mu.Unlock()
	for id, snap := range snaps {
		var s fleetSession
		if json.Unmarshal(snap, &s) != nil {
			continue
		}
		if s.AmbientState != "" {
			w.prevAmbient[id] = s.AmbientState
		} else if id != "" {
			w.prevAmbient[id] = ""
		}
	}
}

// observe takes one freshly-landed snapshot and schedules a wake if it is a
// worker finishing. Called from the session store's onChange, BEFORE the
// visibility filter: a session the shared layout happens to hide is still a
// dispatch that came home, and its manager is still owed the report. (Same
// reasoning fleetSessions gives for reading the full store.)
//
// Non-blocking by construction — it does map work and arms a timer. Every fetch
// and every send happens on the timer's goroutine.
func (w *finishWatcher) observe(ctx context.Context, snap json.RawMessage) {
	var s fleetSession
	if json.Unmarshal(snap, &s) != nil {
		return
	}
	if s.SessionID == "" {
		s.SessionID = snapshotID(snap)
	}
	if s.SessionID == "" {
		return
	}

	w.mu.Lock()
	prev, seen := w.prevAmbient[s.SessionID]
	w.prevAmbient[s.SessionID] = s.AmbientState
	w.mu.Unlock()

	// A first sighting is not a transition. Priming covers the sessions that
	// existed at boot; anything appearing later appears at its own start.
	if !seen || !wasWorking(prev) || s.AmbientState != "idle" {
		return
	}
	parentID := s.ParentSessionID
	if parentID == "" || parentID == s.SessionID {
		return
	}
	// Cheap trigger-time gate — the authoritative one runs again at delivery.
	// It is here only so a finish with no manager behind it never arms a timer.
	all := w.reg.fleetSessions(ctx)
	parent, ok := findFleetSession(all, parentID)
	if !ok || parent.ended() || !parent.IsWakeTarget {
		return
	}
	w.schedule(ctx, parentID, s.SessionID)
}

// schedule adds one worker to its parent's coalesce window, opening the window
// if this is the first finish in it.
func (w *finishWatcher) schedule(ctx context.Context, parentID, workerID string) {
	w.mu.Lock()
	defer w.mu.Unlock()
	for _, id := range w.pending[parentID] {
		if id == workerID {
			return // already in this window; the delivery re-reads it anyway
		}
	}
	w.pending[parentID] = append(w.pending[parentID], workerID)
	if _, open := w.timers[parentID]; open {
		return
	}
	t := w.after(w.coalesce, func() { w.flush(ctx, parentID) })
	w.timers[parentID] = t
}

// flush closes one parent's coalesce window and delivers.
func (w *finishWatcher) flush(ctx context.Context, parentID string) {
	w.mu.Lock()
	workers := w.pending[parentID]
	delete(w.pending, parentID)
	delete(w.timers, parentID)
	w.mu.Unlock()
	if len(workers) == 0 {
		return
	}
	w.sendFinished(ctx, parentID, workers)
}

// forgetWorker drops a worker's dedup signature and its remembered ambient
// state. Called when a row is forgotten (agents.close), so neither map retains
// an entry per session for the process lifetime — the same concern the desktop's
// forgetWorker has. A respawn onto a reused id starts fresh.
func (w *finishWatcher) forgetWorker(sessionID string) {
	w.mu.Lock()
	delete(w.lastReported, sessionID)
	delete(w.prevAmbient, sessionID)
	w.mu.Unlock()
}

// sendFinished composes and delivers one coalesced wake, re-verifying BOTH ends
// against live state first.
//
// The parent is checked here and not only at trigger time: it can have ended,
// been closed, or been reparented out of manager-hood inside the window, and a
// wake to a session that is no longer a manager is a wake nobody reads. The
// workers are re-read for the reason the desktop names: the working→idle edge
// can lie twice — an idle blip mid-stream (the worker is streaming again by now,
// and reporting it would present a half-done result as final), and a final
// assistant message that lands on the conversation stream AFTER the Stop edge,
// because claudemon keeps tailing briefly.
func (w *finishWatcher) sendFinished(ctx context.Context, parentID string, workerIDs []string) {
	all := w.reg.fleetSessions(ctx)
	parent, ok := findFleetSession(all, parentID)
	if !ok || parent.ended() || !parent.IsWakeTarget {
		return
	}

	var entries []fleetEntry
	// Signatures to BOOK, applied only once the send has actually landed.
	delivered := map[string]string{}
	for _, id := range workerIDs {
		s, ok := findFleetSession(all, id)
		if !ok {
			continue // the row was closed inside the window
		}
		// Genuinely idle? An ended session counts (it is not coming back); an
		// unknown ambient state fails open, matching the desktop.
		if !s.ended() && s.AmbientState != "" && s.AmbientState != "idle" {
			continue
		}
		final := w.reg.workerFinalTurn(ctx, id)
		if !final.hasUserTurn {
			continue // boot idle: this session was never given its task
		}
		e := fleetEntry{Label: s.displayLabel(), SessionID: s.SessionID, Cwd: s.Cwd}
		if s.ended() {
			e.Stopped = true
		}
		reply := final.lastAssistant
		if reply != "" {
			e.LastReply = excerptReply(reply)
			// Carry the complete message only when the excerpt is lossy —
			// otherwise the bullet already IS the whole reply.
			if e.LastReply != strings.TrimSpace(reply) {
				e.FullReply = reply
			}
		}
		if reason, failed := workerFailureReason(s.outOfCredits(), reply); failed {
			e.Failed = reason
		}
		sig := finishSignature(reply, e.Stopped, e.Failed)
		w.mu.Lock()
		dup := w.lastReported[id] == sig
		w.mu.Unlock()
		if dup {
			// Nothing new: this edge produced the exact reply and status already
			// delivered for this worker — a flapping block, a re-derived Stop. A
			// genuinely different reply (the wanted case: a manager sending a
			// worker a second instruction) always changes the signature and
			// still wakes the parent.
			continue
		}
		delivered[id] = sig
		entries = append(entries, e)
	}
	if len(entries) == 0 {
		return
	}

	text := buildFleetMessage(fleetWorkerFinishedHeaderFor(entries), fleetWorkerFinishedTail, entries)
	if err := w.reg.deliverFleetWake(ctx, parentID, text); err != nil {
		// Best-effort: the parent may have ended between the check above and the
		// send. Deliberately do NOT book the signatures — the suppression means
		// "the parent has already been told this", so booking it on a send that
		// FAILED would turn a lost wake into a permanently silenced one. The
		// failure mode has to fall on the side of re-reporting.
		log.Printf("brain: worker-finished wake to %s was not delivered: %v", parentID, err)
		return
	}
	w.mu.Lock()
	for id, sig := range delivered {
		w.lastReported[id] = sig
	}
	w.mu.Unlock()
}

// finishSignature is the "have I already told the parent exactly this?" key.
// TWIN: the `${reply} ${stopped} ${failed}` template in sendFinished.
func finishSignature(reply string, stopped bool, failed string) string {
	bit := "0"
	if stopped {
		bit = "1"
	}
	return reply + " " + bit + " " + failed
}

// ── the catch-up backstop ───────────────────────────────────────────────────

// runBackstop drives sweepMissedFinishes until ctx ends.
func (w *finishWatcher) runBackstop(ctx context.Context) {
	ticker := time.NewTicker(fleetWakeBackstopInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case now := <-ticker.C:
			w.sweepMissedFinishes(ctx, now)
		}
	}
}

// sweepMissedFinishes re-nudges a manager that went dark: for each LIVE, IDLE
// manager, find children that finished AFTER it last acted and long enough ago
// that a normal wake would have landed, and send them under the catch-up header.
//
// The dedup is implicit and exact, which is why this needs no bookkeeping of its
// own: the moment the manager acts on a wake its lastActivity advances PAST the
// child's finish and the condition clears. A catch-up that itself fails simply
// re-fires next sweep. TWIN: sweepMissedFinishes.
func (w *finishWatcher) sweepMissedFinishes(ctx context.Context, now time.Time) {
	all := w.reg.fleetSessions(ctx)
	for _, manager := range all {
		if !manager.IsWakeTarget || manager.ended() || manager.AmbientState != "idle" {
			continue
		}
		var entries []fleetEntry
		for _, c := range all {
			if c.ParentSessionID != manager.SessionID || c.SessionID == manager.SessionID {
				continue
			}
			if c.AmbientState != "idle" && !c.ended() {
				continue
			}
			// The manager has not acted since this child finished…
			if c.LastActivity <= manager.LastActivity {
				continue
			}
			// …and the finish is old enough that a normal wake would have landed.
			if now.Sub(time.UnixMilli(c.LastActivity)) <= fleetMissedWakeGrace {
				continue
			}
			final := w.reg.workerFinalTurn(ctx, c.SessionID)
			// Same no-task gate as the live path: a child idling with no user
			// turn was never given its task — nothing finished, nothing to
			// catch up.
			if !final.hasUserTurn {
				continue
			}
			e := fleetEntry{Label: c.displayLabel(), SessionID: c.SessionID, Cwd: c.Cwd}
			if c.ended() {
				e.Stopped = true
			}
			// The catch-up path must tell finished from died too — a manager
			// that missed the live wake is exactly the one most likely to book a
			// crash as an outcome.
			if reason, failed := workerFailureReason(c.outOfCredits(), final.lastAssistant); failed {
				e.Failed = reason
			}
			entries = append(entries, e)
		}
		if len(entries) == 0 {
			continue
		}
		// Best-effort, like the desktop's: still unreachable simply means the
		// next sweep retries. No signature is booked either — a catch-up is a
		// re-send by definition, and suppressing the next one would re-open the
		// dark-manager hole this exists to close.
		_ = w.reg.deliverFleetWake(ctx, manager.SessionID, buildFleetMessage(fleetCatchUpHeader, fleetCatchUpTail, entries))
	}
}

// ── the one conversation read ───────────────────────────────────────────────

// workerFinal is what a finish wake needs from a worker's transcript, and it is
// only ever two things.
type workerFinal struct {
	// hasUserTurn: the session has received at least one real user/task turn. A
	// session whose conversation holds NO user turn has not been given its task
	// yet (the parent's kickoff is still in flight) — its idle is a BOOT idle,
	// not a finish, and waking the manager about it hands it an empty session to
	// read. TWIN: hasReceivedTask.
	hasUserTurn bool
	// lastAssistant is the newest assistant turn's text — the worker's report,
	// and the same text the failure check reads.
	lastAssistant string
}

// workerFinalTurn fetches one worker's conversation and reduces it to the two
// facts above.
//
// FAILS OPEN on hasUserTurn, matching the desktop's "unknown conversation
// (untracked session) fails OPEN": a fetch that errors must not silence a real
// finish, because the cost of a missed wake (a manager dark forever) is strictly
// worse than the cost of a spurious one (one wasted turn reading an empty
// session). The reply is empty in that case and the bullet simply carries no
// excerpt, which is what the desktop does for a session it cannot re-read.
func (r *registry) workerFinalTurn(ctx context.Context, sessionID string) workerFinal {
	raw, err := r.cm.conversation(ctx, sessionID, nil)
	if err != nil {
		return workerFinal{hasUserTurn: true}
	}
	var snap struct {
		Items []json.RawMessage `json:"items"`
	}
	if json.Unmarshal(raw, &snap) != nil {
		return workerFinal{hasUserTurn: true}
	}
	out := workerFinal{}
	for _, item := range snap.Items {
		var it struct {
			Kind string `json:"kind"`
			Text string `json:"text"`
		}
		if json.Unmarshal(item, &it) != nil {
			continue
		}
		switch it.Kind {
		case "user_message":
			out.hasUserTurn = true
		case "assistant_text":
			// The LAST assistant item, not the trailing run joined: claudemon's
			// conversation store merges consecutive assistant_text items in the
			// retained log, so one item already IS one message — the same thing
			// the desktop's `[...conversation].reverse().find(role==='assistant')`
			// picks up, and taking the last one keeps the two readers agreeing.
			out.lastAssistant = it.Text
		}
	}
	return out
}
