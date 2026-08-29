package main

// THE BLOCKED BROADCAST, HEADLESS — the other half of the headless wake story,
// and the sibling of finishwake.go.
//
// TWIN: apps/desktop/src/main/services/supervisorNudge.ts (onBlock,
// onBlockCleared, broadcastBlock, send) plus the two transition sites that
// drive it, claudeSessionStore's hook path (`:1100`) and its managed-mode path
// (`:1197`).
//
// WHY IT HAD TO EXIST, and why it is not the finish wake. finishwake.go tells a
// manager that a dispatch came HOME. This one tells it that a dispatch is
// STUCK: a worker sitting on an approval prompt or a question is not going to
// finish, so its finish wake is not coming, and on a headless node there is no
// GUI, no dock and no human watching a sidebar. Without this the only thing a
// headless manager ever learns about a blocked worker is that it eventually
// finished — hours later, if a human happens to answer, and never at all if
// nobody does.
//
// THE THREE WAYS IT DIFFERS FROM ITS SIBLING, all deliberate:
//
//  1. IT BROADCASTS. A finish is parent-keyed — it is the parent's dispatch
//     coming home and waking anyone else is noise. A block is not: any live
//     manager can gather the context and surface the decision, and the blocked
//     worker's own parent may itself be busy or gone. So every live
//     isWakeTarget receives it, minus the blocked session itself. MORE THAN ONE
//     LIVE WAKE TARGET IS NORMAL, not an edge case — a superseded manager stays
//     alive and idle while its replacement runs, so two is the ordinary
//     steady state after a handoff.
//  2. ITS WINDOW IS SURVIVE-THIS-LONG, NOT COALESCE. fleetFinishCoalesce
//     BATCHES things that already happened: everything that lands in the window
//     goes out when it closes. fleetBlockDebounce is the opposite — the block
//     must still be OPEN when the timer fires or nothing is sent at all. Most
//     approval prompts clear within seconds (an auto-approve hook, a fast
//     human), and every wake costs a manager a full turn of context; waking it
//     for a block that was about to clear anyway trains the doctrine into "fire
//     one blind approve and stay silent" instead of actually reading the
//     decision.
//  3. IT HAS A CLEAR EDGE. Nothing in the finish path is ever un-done. Here the
//     matching transition OUT of a waiting state cancels the pending wake, and
//     it must not leak: see "the two cancellations" below.
//
// WHERE IT DIFFERS FROM THE DESKTOP, honestly:
//
//   - The desktop CAPTURES the supervisor list when the block starts and
//     broadcasts to that captured list 20s later. This recomputes it at
//     broadcast time, so a manager that ended inside the window is not sent a
//     wake nobody will read, and one that appeared inside it does receive one.
//     (The cheap gate in onEdge still means a block that starts with NO manager
//     anywhere arms nothing — same as the desktop.)
//   - The desktop's supervisorSessionIds() does not filter ended sessions; this
//     does, for the reason sendFinished gives — the daemon answers 409 for an
//     ended recipient and the wake is simply lost.
//   - RE-VERIFICATION AT SEND TIME, which the desktop does not do for blocks
//     because it does not need to: its clear edge comes from an in-process hook
//     it cannot miss. The brain's edges come off claudemon's /events SSE stream,
//     which reconnects and re-seeds, so a clear edge CAN be dropped. Rather
//     than trust the cancellation alone, the broadcast re-reads the blocked
//     session from live state and sends nothing if it is no longer blocked.
//     That also means a stale timer cannot produce a false wake — the state is
//     the authority, the timer is only the schedule.
//
// THE TWO CANCELLATIONS, and why Stop() alone is not one of them. The desktop's
// clearTimeout is total: JavaScript is single-threaded, so a cancelled timer
// cannot already be running. time.Timer.Stop() carries no such promise — it
// returns false when the func is already in flight, and the broadcast would go
// out anyway. So every armed debounce carries a GENERATION, and the timer's own
// goroutine re-checks that the entry it is about to act on is still armed and
// still ITS entry before doing anything. Stop() is still called (it frees the
// runtime timer immediately); the generation is what makes the cancel correct.
//
// WHAT THIS DOES NOT DO. There is no backstop. finishwake.go's sweep works
// because a finish is a permanent fact recoverable from a session's own
// lastActivity; "is blocked right now" is live state with no such trace, and a
// sweep over it would re-broadcast the same open block every two minutes for as
// long as it stayed open. The consequence, stated plainly: a worker that was
// ALREADY blocked when the brain started never wakes anyone, because prime()
// records it as blocked and no transition INTO the state is ever seen. The
// desktop has the identical gap across its own restart.
//
// NO MAP GROWS WITH THE SESSION COUNT. Unlike finishWatcher.prevAmbient (one
// entry per session, pruned only on agents.close), every map here holds an
// entry only while a timer is armed for it: the debounce is deleted when it
// fires, when the block clears, and when the row is forgotten, and a coalesce
// window is deleted when it flushes. A clear edge that never arrives still
// self-cleans 20 seconds later when the debounce fires and re-verification
// drops it. This watcher deliberately shares finishWatcher's prevAmbient rather
// than keeping a second copy of it — one transition, one memory of the state
// before it.

import (
	"context"
	"log"
	"sync"
	"time"
)

const (
	// fleetBlockDebounce is how long a block must SURVIVE before it wakes
	// anyone. NOT a coalesce window: a block that clears inside it is never
	// reported at all. TWIN: BLOCK_DEBOUNCE_MS.
	fleetBlockDebounce = 20 * time.Second

	// fleetBlockCoalesce is how long ONE recipient's blocks are gathered before
	// its wake goes out, so a burst of workers blocking together costs a manager
	// one turn rather than one each. TWIN: COALESCE_MS, which the desktop shares
	// between this path and the finish path — as does fleetFinishCoalesce here.
	fleetBlockCoalesce = 1500 * time.Millisecond
)

// blockWatcher turns "a worker entered a decision point and stayed there" into
// a broadcast to every live manager. Fed by finishWatcher.observe from the same
// ambient transition, so there is exactly one prevAmbient in the process.
type blockWatcher struct {
	reg *registry

	// debounce, coalesce and after are injectable so a test can drive both
	// windows deterministically instead of sleeping through 20 real seconds.
	debounce time.Duration
	coalesce time.Duration
	after    func(time.Duration, func()) *time.Timer

	mu sync.Mutex
	// debounces is the survive-this-long timer per BLOCKED session — one per
	// worker, because a worker can only be blocked on one thing at a time, and
	// that one timer covers every manager it would eventually wake.
	debounces map[string]*blockDebounce
	// gen is the generation counter behind every cancellation. See the header:
	// Stop() cannot promise a timer is not already running, so the closure
	// checks its own generation before acting.
	gen uint64
	// pending is the open coalesce window per RECIPIENT: the blocked session ids
	// gathered into it, in arrival order so a multi-entry wake is deterministic.
	pending map[string][]string
	timers  map[string]*time.Timer
}

// blockDebounce is one armed survive-this-long timer and the generation that
// makes cancelling it correct.
type blockDebounce struct {
	gen   uint64
	timer *time.Timer
}

func newBlockWatcher(reg *registry) *blockWatcher {
	return &blockWatcher{
		reg:       reg,
		debounce:  fleetBlockDebounce,
		coalesce:  fleetBlockCoalesce,
		after:     time.AfterFunc,
		debounces: map[string]*blockDebounce{},
		pending:   map[string][]string{},
		timers:    map[string]*time.Timer{},
	}
}

// isBlockedAmbient reports whether an ambient state is a decision point a
// manager should be told about. TWIN: the `isBlocked` closure both desktop
// transition sites define.
//
// An EMPTY ambient state is not blocked, and that is load-bearing in the clear
// direction: enrich.go omits the field for a mode this vocabulary cannot
// express (claudemon's `unknown`), so waiting_approval → unknown reads here as
// a clear. That is the conservative direction — it costs at most a wake that
// send-time re-verification would have dropped anyway, since re-verification
// asks this same question.
func isBlockedAmbient(ambient string) bool {
	return ambient == "waiting_approval" || ambient == "waiting_input"
}

// blockedOnKind is the word the bullet renders in place of a cwd.
// TWIN: `session.pendingApproval ? 'approval' : 'question'` on the hook path and
// `next === 'waiting_approval' ? 'approval' : 'question'` on the managed one —
// the same two answers, read off the ambient state the brain already has.
func blockedOnKind(ambient string) string {
	if ambient == "waiting_approval" {
		return "approval"
	}
	return "question"
}

// onEdge takes one ambient transition and arms, cancels, or ignores.
//
// The rule is the desktop's exactly: INTO a blocked state from a non-blocked one
// arms; OUT of one to a non-blocked one cancels; everything else — including
// approval → question, which is still one continuous block — does nothing. That
// last case is why a single long block broadcasts exactly ONCE: only a
// transition into the state re-arms, and the session has to leave first.
//
// A session's FIRST sighting can arm. `prev` is empty for a row we have never
// seen, which is not blocked, so a session that appears already blocked reads as
// an edge. That is the opposite of the finish rule (where a first sighting must
// never read as a finish) and it is right for the same underlying reason: idle
// is the resting state every new session appears in, while blocked is not — a
// session sighted blocked genuinely IS blocked, and the debounce plus the
// send-time re-verify make arming on it free if it was a race.
func (b *blockWatcher) onEdge(ctx context.Context, s fleetSession, prev string) {
	now := s.AmbientState
	switch {
	case isBlockedAmbient(now) && !isBlockedAmbient(prev):
		b.onBlocked(ctx, s.SessionID)
	case !isBlockedAmbient(now) && isBlockedAmbient(prev):
		b.onBlockCleared(s.SessionID)
	}
}

// onBlocked arms (or re-arms) the survive-this-long timer for one blocked
// session.
//
// The cheap trigger-time gate is here for the reason finishWatcher.observe's
// is: so a block with no manager anywhere behind it never arms a timer. It is
// NOT the authoritative recipient list — that is recomputed when the timer
// fires, because 20 seconds is long enough for a manager to end or a successor
// to appear.
func (b *blockWatcher) onBlocked(ctx context.Context, sessionID string) {
	if sessionID == "" {
		return
	}
	if !b.anyWakeTargetBesides(ctx, sessionID) {
		return // no manager → this wake is optional, nothing to do
	}

	b.mu.Lock()
	// Re-blocking after a clear, or a second edge before the debounce fires,
	// REPLACES the armed timer rather than stacking one — a flapping block can
	// neither leak a timer nor double-fire.
	if prev, ok := b.debounces[sessionID]; ok {
		prev.timer.Stop()
	}
	b.gen++
	gen := b.gen
	d := &blockDebounce{gen: gen}
	b.debounces[sessionID] = d
	b.mu.Unlock()

	d.timer = b.after(b.debounce, func() {
		b.mu.Lock()
		// The generation check IS the cancellation (see the header): Stop()
		// cannot promise this func is not already running, so a timer that was
		// cancelled or replaced while in flight must notice and do nothing.
		cur, ok := b.debounces[sessionID]
		if !ok || cur.gen != gen {
			b.mu.Unlock()
			return
		}
		delete(b.debounces, sessionID)
		b.mu.Unlock()
		b.broadcast(ctx, sessionID)
	})
}

// onBlockCleared cancels a pending wake for a block that resolved before it had
// to be anyone's problem. A no-op when nothing is armed (the block already
// survived and broadcast, or there never was one), so it is safe to call on
// every un-block edge. TWIN: onBlockCleared.
func (b *blockWatcher) onBlockCleared(sessionID string) {
	b.mu.Lock()
	d, ok := b.debounces[sessionID]
	if ok {
		delete(b.debounces, sessionID)
		d.timer.Stop()
	}
	b.mu.Unlock()
}

// forget drops every trace of one session: its armed debounce, its own coalesce
// window if it is a recipient, and its entry in anyone else's window.
//
// Called from finishWatcher.forgetWorker (agents.close), which is the one place
// a row leaves the store without a transition. Without it a session dismissed
// while blocked would leave an armed timer behind for up to 20 seconds — it
// would broadcast nothing (re-verification finds no row), but the entry should
// go with the row it belongs to rather than expire on its own.
func (b *blockWatcher) forget(sessionID string) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if d, ok := b.debounces[sessionID]; ok {
		delete(b.debounces, sessionID)
		d.timer.Stop()
	}
	delete(b.pending, sessionID)
	delete(b.timers, sessionID)
	for recipient, ids := range b.pending {
		kept := ids[:0]
		for _, id := range ids {
			if id != sessionID {
				kept = append(kept, id)
			}
		}
		if len(kept) == 0 {
			// Leave no empty key behind. The window's timer stays armed and
			// flush() will find nothing and delete it — an empty entry here
			// would just be a map key with no meaning until then.
			delete(b.pending, recipient)
			continue
		}
		b.pending[recipient] = kept
	}
}

// anyWakeTargetBesides is the cheap gate: is there any live manager at all that
// is not the blocked session itself?
func (b *blockWatcher) anyWakeTargetBesides(ctx context.Context, sessionID string) bool {
	for _, s := range b.reg.fleetSessions(ctx) {
		if s.IsWakeTarget && !s.ended() && s.SessionID != sessionID {
			return true
		}
	}
	return false
}

// broadcast runs once a block has SURVIVED the debounce: it re-verifies the
// block against live state, resolves the recipients, and adds the blocked
// session to each recipient's coalesce window.
//
// The re-verification is the guard that makes a dropped clear edge harmless —
// an SSE reconnect, a re-seed, a row that vanished. The timer says WHEN to
// look; the store says WHETHER there is anything to say.
func (b *blockWatcher) broadcast(ctx context.Context, blockedID string) {
	all := b.reg.fleetSessions(ctx)
	blocked, ok := findFleetSession(all, blockedID)
	if !ok || blocked.ended() || !isBlockedAmbient(blocked.AmbientState) {
		return // it cleared, ended or vanished — nothing to report
	}

	b.mu.Lock()
	defer b.mu.Unlock()
	for _, m := range all {
		// A manager is never told about its OWN block: it cannot gather context
		// on a decision it is itself sitting on.
		if !m.IsWakeTarget || m.ended() || m.SessionID == blockedID {
			continue
		}
		b.addLocked(ctx, m.SessionID, blockedID)
	}
}

// addLocked puts one blocked session into one recipient's window, opening the
// window if this is the first entry in it. Caller holds b.mu.
func (b *blockWatcher) addLocked(ctx context.Context, recipientID, blockedID string) {
	for _, id := range b.pending[recipientID] {
		if id == blockedID {
			return // already in this window; delivery re-reads it anyway
		}
	}
	b.pending[recipientID] = append(b.pending[recipientID], blockedID)
	if _, open := b.timers[recipientID]; open {
		return
	}
	b.timers[recipientID] = b.after(b.coalesce, func() { b.flush(ctx, recipientID) })
}

// flush closes one recipient's coalesce window and delivers its wake.
//
// Each recipient has its OWN window and its OWN send, which is what makes the
// fan-out failure-isolated by construction: one unreachable manager cannot
// swallow, delay or suppress another's copy of the same broadcast. And unlike
// the finish wake there is no signature to book, so a failed send cannot
// silence anything either — the next real block edge arms from scratch.
func (b *blockWatcher) flush(ctx context.Context, recipientID string) {
	b.mu.Lock()
	blockedIDs := b.pending[recipientID]
	delete(b.pending, recipientID)
	delete(b.timers, recipientID)
	b.mu.Unlock()
	if len(blockedIDs) == 0 {
		return
	}
	b.send(ctx, recipientID, blockedIDs)
}

// send composes and delivers one recipient's wake, re-verifying BOTH ends
// against live state first — the same rule sendFinished states, for the same
// reason: 1.5 seconds is long enough for the recipient to end and for a block
// to be answered.
func (b *blockWatcher) send(ctx context.Context, recipientID string, blockedIDs []string) {
	all := b.reg.fleetSessions(ctx)
	recipient, ok := findFleetSession(all, recipientID)
	if !ok || recipient.ended() || !recipient.IsWakeTarget {
		return
	}

	var entries []fleetEntry
	for _, id := range blockedIDs {
		s, ok := findFleetSession(all, id)
		if !ok || s.ended() || !isBlockedAmbient(s.AmbientState) {
			continue // answered, ended or closed inside the window
		}
		// A blocked bullet carries no cwd: the parser's `where` slot holds
		// EITHER a cwd or the block kind, never both.
		entries = append(entries, fleetEntry{
			Label:     s.displayLabel(),
			SessionID: s.SessionID,
			BlockedOn: blockedOnKind(s.AmbientState),
		})
	}
	if len(entries) == 0 {
		return
	}

	text := buildFleetMessage(fleetBlockedHeader, fleetBlockedTail, entries)
	if err := b.reg.deliverFleetWake(ctx, recipientID, text); err != nil {
		// Best-effort, and deliberately terminal for THIS recipient only: the
		// manager may have ended between the check above and the send. Nothing
		// is booked, so the next block edge is unaffected.
		log.Printf("brain: blocked broadcast to %s was not delivered: %v", recipientID, err)
	}
}
