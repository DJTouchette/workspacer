package main

// The blocked broadcast, headless: the transition that arms it, the
// survive-this-long window that is NOT a coalesce window, the clear edge that
// un-does it and must leave nothing behind, and the fan-out to every live
// manager rather than to one parent.
//
// Same harness as its sibling (wakeRig, finishwake_test.go) and the same
// discipline — drive the watcher the way production does, by pushing claudemon
// rows through the store, then close each window explicitly instead of sleeping
// through 20 real seconds.
//
// A NOTE ON THE CANCELLATION CASES. The rig's fake timer ignores Stop(), so
// every "this must not fire" case below fires the stale closure anyway. That is
// deliberate: time.Timer.Stop() makes no promise about a func already in flight,
// so a cancellation that only worked because Stop() worked would be one Go does
// not actually guarantee. What these cases pin is that the watcher's OWN state
// (the generation guard, and the re-verification at broadcast) refuses it.

import (
	"encoding/json"
	"strings"
	"testing"
	"time"
)

// blockFleet is the shape every case starts from: one manager, one worker
// mid-turn. Deliberately NOT r.fleet() — a blocked worker's conversation is
// irrelevant here (the bullet carries no reply), and starting from "responding"
// is what makes the next update a real transition.
func blockFleet(r *wakeRig) {
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{Label: "rust worker", ParentSessionID: "mgr"})
	r.update("mgr", "/work", "input")
	r.update("w1", "/work/proj", "responding")
}

// surviveDebounce is the 20 seconds passing with the block still open.
func surviveDebounce(r *wakeRig) { r.fire("block-debounce") }

// deliver closes the per-recipient coalesce windows the debounce just opened.
func deliver(r *wakeRig) { r.fire("block-coalesce") }

// ── the broadcast itself ────────────────────────────────────────────────────

// THE WHOLE POINT, and the thing that makes this not the finish wake: a block
// that survives goes to EVERY live manager, not to the blocked worker's parent.
// Two live wake targets is the ordinary steady state after a handoff — a
// superseded manager stays alive and idle while its replacement runs — so this
// is the normal case, not an edge case.
func TestASurvivingBlockIsBroadcastToEveryLiveManager(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	// The superseded manager: still alive, still a wake target, not w1's parent.
	r.spawnMetaFor("mgr2", spawnMeta{Label: "Manager Two", IsWakeTarget: true})
	r.update("mgr2", "/elsewhere", "input")

	r.update("w1", "/work/proj", "approval")
	if n := r.count("block-debounce"); n != 1 {
		t.Fatalf("the block armed %d debounce timers, want 1", n)
	}
	surviveDebounce(r)
	deliver(r)

	for _, id := range []string{"mgr", "mgr2"} {
		wakes := r.d.to(id)
		if len(wakes) != 1 {
			t.Fatalf("%s received %d wakes, want 1 — a broadcast reaches every live manager, not just the parent", id, len(wakes))
		}
		if !strings.HasPrefix(wakes[0], fleetBlockedHeader) {
			t.Errorf("%s's wake does not open with the blocked header (every client's card parser keys off it):\n%s", id, wakes[0])
		}
		if !strings.Contains(wakes[0], "- rust worker (session:w1, approval)") {
			t.Errorf("%s's bullet is wrong — a blocked entry spends the `where` slot on the block kind, not a cwd:\n%s", id, wakes[0])
		}
		if !strings.HasSuffix(wakes[0], fleetBlockedTail) {
			t.Errorf("%s's wake lost its instruction tail:\n%s", id, wakes[0])
		}
	}
	// The blocked session is never told about its own block.
	if n := len(r.d.to("w1")); n != 0 {
		t.Errorf("the blocked session received %d wakes about itself", n)
	}
}

// A blocked MANAGER is excluded from its own broadcast but not from anyone
// else's: it cannot gather context on the decision it is itself sitting on.
func TestABlockedManagerIsNotToldAboutItsOwnBlock(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.spawnMetaFor("mgr2", spawnMeta{Label: "Manager Two", IsWakeTarget: true})
	r.update("mgr2", "/elsewhere", "input")

	r.update("mgr", "/work", "question") // the manager itself is now blocked
	surviveDebounce(r)
	deliver(r)

	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a blocked manager was woken about its own block (%d wakes)", n)
	}
	wakes := r.d.to("mgr2")
	if len(wakes) != 1 {
		t.Fatalf("the other manager received %d wakes, want 1", len(wakes))
	}
	if !strings.Contains(wakes[0], "(session:mgr, question)") {
		t.Errorf("the bullet lost the blocked session or its kind:\n%s", wakes[0])
	}
}

// A block is NOT parent-keyed. A worker nobody dispatched — no parent at all —
// still blocks a decision somebody has to make, and every manager can make it.
// This is the exact case the finish wake drops on the floor.
func TestAnUnparentedBlockStillBroadcasts(t *testing.T) {
	r := newWakeRig(t)
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.update("mgr", "/work", "input")
	r.update("solo", "/work/solo", "responding")

	r.update("solo", "/work/solo", "approval")
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("an unparented block produced %d wakes, want 1", n)
	}
}

// The bullet renders the kind the ambient state actually says. approval and
// question are the parser's two alternatives; anything else is unparseable.
func TestTheBulletNamesWhichKindOfBlockItIs(t *testing.T) {
	for mode, want := range map[string]string{"approval": "approval", "question": "question"} {
		r := newWakeRig(t)
		blockFleet(r)
		r.update("w1", "/work/proj", mode)
		surviveDebounce(r)
		deliver(r)
		wakes := r.d.to("mgr")
		if len(wakes) != 1 {
			t.Fatalf("%s: got %d wakes, want 1", mode, len(wakes))
		}
		if !strings.Contains(wakes[0], "(session:w1, "+want+")") {
			t.Errorf("%s: the bullet did not name the block kind:\n%s", mode, wakes[0])
		}
	}
}

// ── survive-this-long is NOT a coalesce ─────────────────────────────────────

// THE DIFFERENCE FROM THE FINISH WAKE, pinned. fleetFinishCoalesce BATCHES
// things that already happened: everything in the window goes out when it
// closes. fleetBlockDebounce is the opposite — a block that clears inside the
// window is never reported at all. Most approval prompts clear within seconds,
// and a manager woken for one of those pays a full turn of context to read a
// decision that no longer exists.
func TestABlockThatClearsInsideTheWindowWakesNobody(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)

	r.update("w1", "/work/proj", "approval")   // blocked…
	r.update("w1", "/work/proj", "responding") // …and answered, 3 seconds later

	// The stale closure is fired anyway (the rig ignores Stop): the CANCEL has
	// to be state, not a timer that may already be running.
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a block that cleared inside its window still woke the manager (%d wakes) — this window is survive-this-long, not a coalesce", n)
	}

	// Contrast, in the same test so the difference cannot drift apart: a block
	// that DOES survive the identical window is reported.
	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("a block that survived the window produced %d wakes, want 1", n)
	}
}

// A block that clears AFTER the debounce but before delivery is still not
// reported. The desktop does not need this check — its clear edge comes from an
// in-process hook it cannot miss — but the brain's edges come off a reconnecting
// SSE stream, so the store, not the timer, is the authority.
func TestABlockAnsweredBeforeDeliveryIsNotReported(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r)                         // the broadcast is composed…
	r.update("w1", "/work/proj", "responding") // …and answered inside the 1.5s window
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a block answered inside the coalesce window was still reported (%d wakes)", n)
	}
}

// The clear edge that no snapshot carries: a DROPPED one. If the row simply
// vanishes (an /events reconnect, a re-seed), the debounce still fires — and
// re-verification finds nothing to say. A stale timer can never invent a wake.
func TestABlockWhoseRowVanishedBroadcastsNothing(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	r.store.remove("w1")
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a block whose session no longer exists was broadcast (%d wakes)", n)
	}
}

// The clear edge the brain NEVER SAW. Its edges come off a reconnecting SSE
// stream, so a block can be answered across the gap and no transition ever
// arrives. The debounce fires on schedule and re-verification finds a session
// that is working again — the STORE is the authority, the timer is only the
// schedule. Without this arm the cancellation would depend on an edge the brain
// cannot promise it received.
func TestABlockClearedByAnEdgeWeNeverSawIsNotBroadcast(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	r.updateUnobserved("w1", "/work/proj", "responding") // answered across an SSE gap
	surviveDebounce(r)
	// Caught at the BROADCAST, before any recipient window is opened. The
	// send-time check would refuse the entry too, but only after arming one
	// window per manager and walking the store again for each — this is the
	// cheap answer to a question already settled.
	if n := r.count("block-coalesce"); n != 0 {
		t.Errorf("a block that is no longer open still opened %d recipient windows", n)
	}
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a block whose clear edge was dropped was still broadcast (%d wakes)", n)
	}
}

// approval → question is ONE continuous block, not two. Only a transition INTO
// a blocked state from a non-blocked one re-arms, so a long block broadcasts
// exactly once however the daemon re-spells it.
func TestAContinuousBlockBroadcastsExactlyOnce(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	r.update("w1", "/work/proj", "question") // still blocked, differently spelled
	if n := r.count("block-debounce"); n != 1 {
		t.Fatalf("a continuous block armed %d debounce timers, want 1", n)
	}
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("a continuous block produced %d wakes, want 1", n)
	}
}

// …but a block that clears and RE-blocks is a genuinely new edge and wakes
// again. Nothing here books a suppression signature, so a flapping block
// re-arms from scratch every time.
func TestAReBlockAfterAClearWakesAgain(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r)
	deliver(r)
	r.update("w1", "/work/proj", "responding")
	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 2 {
		t.Errorf("a re-block produced %d wakes total, want 2", n)
	}
}

// A flapping block must not STACK timers either: re-arming replaces, and the
// superseded closure is refused by its generation when it fires late.
func TestAFlappingBlockNeitherStacksTimersNorDoubleFires(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	for i := 0; i < 3; i++ {
		r.update("w1", "/work/proj", "approval")
		r.update("w1", "/work/proj", "responding")
	}
	r.update("w1", "/work/proj", "approval") // the one that sticks

	// Four arm edges happened; four stale closures are now fired at once. Only
	// the newest generation may act.
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("a flapping block produced %d wakes, want exactly 1", n)
	}
	if n := r.fin.blocks.armed(); n != 0 {
		t.Errorf("%d debounce entries survived the flap — a replaced timer must not leak its entry", n)
	}
}

// ── the clear edge must leave nothing behind ────────────────────────────────

// A block that resolves leaves no state at all: no armed debounce, and nothing
// in any coalesce window. The maps here are the ONE thing in this watcher that
// could grow without bound, and unlike finishWatcher.prevAmbient (one entry per
// session, pruned on agents.close) they hold an entry only while a timer is
// armed for it.
func TestAResolvedBlockLeavesNoStateBehind(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)

	// Cleared inside the window.
	r.update("w1", "/work/proj", "approval")
	r.update("w1", "/work/proj", "responding")
	if n := r.fin.blocks.armed(); n != 0 {
		t.Errorf("a cleared block left %d armed debounces", n)
	}

	// Survived, delivered, done.
	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r)
	if n := r.fin.blocks.armed(); n != 0 {
		t.Errorf("a fired debounce left %d entries behind — it must delete its own", n)
	}
	deliver(r)
	if open, windows := r.fin.blocks.windows(); open != 0 || windows != 0 {
		t.Errorf("delivery left %d pending lists and %d window timers behind", open, windows)
	}
}

// A session that DIES while blocked clears through the ordinary edge: claudemon
// reports mode "stopped", which reads as ambient idle, which is not blocked.
func TestASessionThatDiesWhileBlockedCancelsItsWake(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	r.update("w1", "/work/proj", "stopped")
	if n := r.fin.blocks.armed(); n != 0 {
		t.Errorf("a session that died while blocked left %d armed debounces", n)
	}
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a dead session's block was broadcast (%d wakes)", n)
	}
}

// A session DISMISSED while blocked is the one path with no transition at all —
// agents.close removes the row silently. forgetWorker has to take the block
// state with it.
func TestForgettingASessionCancelsItsPendingBlock(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	if n := r.fin.blocks.armed(); n != 1 {
		t.Fatalf("the block armed %d debounces, want 1", n)
	}
	r.fin.forgetWorker("w1")
	if n := r.fin.blocks.armed(); n != 0 {
		t.Errorf("forgetting a blocked session left %d armed debounces", n)
	}
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a forgotten session's block was broadcast (%d wakes)", n)
	}
}

// Forgetting a session that is already inside a coalesce window drops it from
// there too, rather than waiting for delivery to re-verify it away.
func TestForgettingASessionDropsItFromAnOpenWindow(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r) // w1 is now in mgr's open window
	if open, _ := r.fin.blocks.windows(); open != 1 {
		t.Fatalf("the broadcast opened %d windows, want 1", open)
	}
	r.fin.forgetWorker("w1")
	if open, _ := r.fin.blocks.windows(); open != 0 {
		t.Errorf("forgetting the blocked session left it in %d open windows", open)
	}
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a forgotten session was still delivered (%d wakes)", n)
	}
}

// ── the fan-out ─────────────────────────────────────────────────────────────

// A partial failure must not silence the rest. One manager unreachable (its
// session ended between the check and the send) is one 409; every other
// recipient still gets its copy, because each has its own window and its own
// send.
func TestOneUnreachableManagerDoesNotSilenceTheOthers(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.spawnMetaFor("mgr2", spawnMeta{Label: "Manager Two", IsWakeTarget: true})
	r.spawnMetaFor("mgr3", spawnMeta{Label: "Manager Three", IsWakeTarget: true})
	r.update("mgr2", "/two", "input")
	r.update("mgr3", "/three", "input")
	r.d.mu.Lock()
	r.d.refuse["mgr2"] = true
	r.d.mu.Unlock()

	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r)
	deliver(r)

	for _, id := range []string{"mgr", "mgr3"} {
		if n := len(r.d.to(id)); n != 1 {
			t.Errorf("%s received %d wakes — one refusing recipient swallowed another's copy", id, n)
		}
	}
	if n := len(r.d.to("mgr2")); n != 0 {
		t.Errorf("the refusing recipient recorded %d wakes", n)
	}
}

// …and a failed send books nothing, so the NEXT real block edge is unaffected.
// (This wake keeps no suppression signature at all — unlike the finish wake,
// which has one and must not book it on a send that threw.)
func TestAFailedBroadcastDoesNotSuppressTheNextBlock(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.d.mu.Lock()
	r.d.refuse["mgr"] = true
	r.d.mu.Unlock()

	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Fatalf("the refused send was recorded as delivered (%d)", n)
	}

	r.d.mu.Lock()
	r.d.refuse["mgr"] = false
	r.d.mu.Unlock()
	r.update("w1", "/work/proj", "responding")
	r.update("w1", "/work/proj", "approval") // the same block, again
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("the next real block edge was suppressed by a wake that never landed (%d wakes)", n)
	}
}

// Two workers blocking together cost a manager ONE turn, not two. This is the
// coalescing layer, and it is per recipient — the same rule the finish wake
// applies per parent.
func TestBlocksArrivingTogetherCoalesceIntoOneWake(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.spawnMetaFor("w2", spawnMeta{Label: "go worker", ParentSessionID: "mgr"})
	r.update("w2", "/work/other", "responding")

	r.update("w1", "/work/proj", "approval")
	r.update("w2", "/work/other", "question")
	surviveDebounce(r)
	if _, windows := r.fin.blocks.windows(); windows != 1 {
		t.Fatalf("two blocks for one manager opened %d windows, want 1", windows)
	}
	deliver(r)

	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("got %d wakes, want 1 carrying both blocks", len(wakes))
	}
	if !strings.Contains(wakes[0], "(session:w1, approval)") || !strings.Contains(wakes[0], "(session:w2, question)") {
		t.Errorf("the coalesced wake lost an entry:\n%s", wakes[0])
	}
}

// ── who is a recipient ──────────────────────────────────────────────────────

// No manager anywhere → nothing is armed. The broadcast is optional machinery:
// on a node running no Fleet Manager it must cost nothing at all.
func TestABlockWithNoManagerAnywhereArmsNothing(t *testing.T) {
	r := newWakeRig(t)
	r.spawnMetaFor("plain", spawnMeta{Label: "not a manager"})
	r.spawnMetaFor("w1", spawnMeta{ParentSessionID: "plain"})
	r.update("plain", "/work", "input")
	r.update("w1", "/work/p", "responding")

	r.update("w1", "/work/p", "approval")
	if n := r.count("block-debounce"); n != 0 {
		t.Errorf("a block with no manager anywhere armed %d timers", n)
	}
	if n := len(r.d.to("plain")); n != 0 {
		t.Errorf("a non-manager received %d wakes", n)
	}
}

// A manager that ENDED inside the 20-second window is not sent a wake nobody
// will read. The recipient list is resolved at broadcast time, not captured
// when the block started — which is where this diverges from the desktop.
func TestAManagerThatEndedInsideTheWindowIsNotBroadcastTo(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.spawnMetaFor("mgr2", spawnMeta{Label: "Manager Two", IsWakeTarget: true})
	r.update("mgr2", "/elsewhere", "input")

	r.update("w1", "/work/proj", "approval")
	r.update("mgr", "/work", "stopped") // the manager died inside the window
	surviveDebounce(r)
	deliver(r)

	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("an ended manager was broadcast to (%d wakes)", n)
	}
	if n := len(r.d.to("mgr2")); n != 1 {
		t.Errorf("the surviving manager received %d wakes, want 1", n)
	}
}

// The RECIPIENT is re-checked at delivery too, not only when the broadcast is
// composed: 1.5 seconds is long enough for a manager to end, and a wake to an
// ended session is a guaranteed 409. TWIN in spirit:
// TestAManagerThatEndedInsideTheWindowIsNotWoken on the finish path.
func TestAManagerThatEndedInsideTheCoalesceWindowIsNotSentTo(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.spawnMetaFor("mgr2", spawnMeta{Label: "Manager Two", IsWakeTarget: true})
	r.update("mgr2", "/elsewhere", "input")

	r.update("w1", "/work/proj", "approval")
	surviveDebounce(r)                  // both managers now have an open window
	r.update("mgr", "/work", "stopped") // …and one of them dies inside it
	deliver(r)

	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a manager that ended inside the coalesce window was sent to (%d wakes)", n)
	}
	if n := len(r.d.to("mgr2")); n != 1 {
		t.Errorf("the surviving manager received %d wakes, want 1", n)
	}
}

// The mirror: a manager that APPEARED inside the window still receives the
// broadcast, because the list is resolved when the timer fires.
func TestAManagerThatAppearedInsideTheWindowStillReceivesTheBroadcast(t *testing.T) {
	r := newWakeRig(t)
	blockFleet(r)
	r.update("w1", "/work/proj", "approval")
	r.spawnMetaFor("mgr2", spawnMeta{Label: "Successor", IsWakeTarget: true})
	r.update("mgr2", "/elsewhere", "input") // spawned mid-window
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr2")); n != 1 {
		t.Errorf("a manager that appeared inside the window received %d wakes, want 1", n)
	}
}

// A worker sighted for the FIRST time already blocked is genuinely blocked, so
// it arms — the opposite of the finish rule, where a first sighting must never
// read as a finish. Idle is the resting state every new session appears in;
// blocked is not.
func TestASessionSightedAlreadyBlockedStillArms(t *testing.T) {
	r := newWakeRig(t)
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{Label: "worker", ParentSessionID: "mgr"})
	r.update("mgr", "/work", "input")
	r.update("w1", "/work/p", "approval") // the very first row we ever see for w1
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 1 {
		t.Errorf("a session first sighted blocked produced %d wakes, want 1", n)
	}
}

// …but a session that was ALREADY blocked when the brain booted does not, and
// this pins that known gap rather than leaving it to be discovered. prime()
// records it as blocked, so no transition INTO the state is ever seen and
// nothing wakes. There is no backstop for this: "is blocked right now" is live
// state with no lastActivity trace to sweep, and a sweep over it would
// re-broadcast the same open block every two minutes. The desktop has the
// identical gap across its own restart.
func TestABlockAlreadyOpenAtBootWakesNobody(t *testing.T) {
	r := newWakeRig(t)
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.spawnMetaFor("w1", spawnMeta{Label: "worker", ParentSessionID: "mgr"})
	r.store.seed(map[string]json.RawMessage{
		"mgr": json.RawMessage(`{"session_id":"mgr","cwd":"/work","mode":"input"}`),
		"w1":  json.RawMessage(`{"session_id":"w1","cwd":"/work/p","mode":"approval"}`),
	})
	// A redundant snapshot of the same state is not a transition either.
	r.update("w1", "/work/p", "approval")
	surviveDebounce(r)
	deliver(r)
	if n := len(r.d.to("mgr")); n != 0 {
		t.Errorf("a block already open at boot produced %d wakes — if this now fires, the gap was closed and this test should say so", n)
	}
}

// ── the wire format ─────────────────────────────────────────────────────────

// The composed wake, whole. The bullet grammar's `where` slot is an ALTERNATION
// on the parse side, so a blocked entry that also carried a cwd would be
// unparseable rather than merely verbose.
func TestTheBlockedWakeIsTheDesktopsExactShape(t *testing.T) {
	got := buildFleetMessage(fleetBlockedHeader, fleetBlockedTail, []fleetEntry{
		{Label: "rust worker", SessionID: "w1", BlockedOn: "approval"},
		{Label: "go worker", SessionID: "w2", BlockedOn: "question"},
	})
	want := fleetBlockedHeader + "\n" +
		"- rust worker (session:w1, approval)\n" +
		"- go worker (session:w2, question)\n" +
		fleetBlockedTail
	if got != want {
		t.Errorf("blocked wake\n got %q\nwant %q", got, want)
	}
}

// The label falls back the way every other bullet's does: the session's own
// name, else its cwd basename, else "Agent".
func TestABlockedWorkerWithNoLabelIsNamedByItsCwd(t *testing.T) {
	r := newWakeRig(t)
	r.spawnMetaFor("mgr", spawnMeta{Label: "Fleet Manager", IsWakeTarget: true})
	r.update("mgr", "/work", "input")
	r.update("w1", "/work/some-project", "responding")
	r.update("w1", "/work/some-project", "approval")
	surviveDebounce(r)
	deliver(r)
	wakes := r.d.to("mgr")
	if len(wakes) != 1 {
		t.Fatalf("got %d wakes, want 1", len(wakes))
	}
	if !strings.Contains(wakes[0], "- some-project (session:w1, approval)") {
		t.Errorf("an unlabelled worker was not named by its cwd basename:\n%s", wakes[0])
	}
}

// ── the constants ───────────────────────────────────────────────────────────

// The debounce is the desktop's BLOCK_DEBOUNCE_MS. It is the number that
// decides whether a manager reads real decisions or is trained to fire blind
// approvals, so it is pinned rather than left to drift.
func TestTheBlockDebounceMatchesTheDesktop(t *testing.T) {
	if fleetBlockDebounce != 20*time.Second {
		t.Errorf("fleetBlockDebounce is %s, want 20s (BLOCK_DEBOUNCE_MS)", fleetBlockDebounce)
	}
	if fleetBlockCoalesce != fleetFinishCoalesce {
		t.Errorf("the block coalesce window (%s) drifted from the finish one (%s) — the desktop shares one COALESCE_MS between them",
			fleetBlockCoalesce, fleetFinishCoalesce)
	}
}

// ── introspection helpers (test-only) ───────────────────────────────────────

// armed is how many debounce timers are currently held. Used only to prove the
// maps do not leak.
func (b *blockWatcher) armed() int {
	b.mu.Lock()
	defer b.mu.Unlock()
	return len(b.debounces)
}

// windows is how many recipient entry-lists and window timers are held.
func (b *blockWatcher) windows() (int, int) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return len(b.pending), len(b.timers)
}
