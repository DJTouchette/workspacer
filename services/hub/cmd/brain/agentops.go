package main

// THE FLEET MANAGER'S OWN TOOLBOX, headless: agents.reportProgress,
// agents.notifyWhen, agents.close, agents.orphans, agents.reparent.
//
// TWIN: the corresponding registerCapability blocks in
// apps/desktop/src/main/services/hubCapabilities.ts, backed by
// services/progressReports.ts, services/thresholdWatch.ts and
// claudeSessionStore's closeSession / orphanCandidates / reparentChildren.
//
// WHY THIS IS THE PART THAT MATTERED. Every one of these is desktop-main-only,
// so a Fleet Manager running on a headless node lost the ability to be told a
// worker is going wrong, to watch one for runaway cost, to dismiss a finished
// one, to adopt the workers of a manager that died, or — with brief.go — to
// write anything down. Those failures are QUIET: the manager keeps working and
// simply stops seeing things. That is worse than a manager that cannot start.
//
// TIER, for each, checked against internal/authtoken rather than assumed:
//
//   - agents.reportProgress is ALREADY in viewMethods, admitted there because
//     it names no recipient (the host derives it) and cannot name a sender
//     either (the router strips callerSessionId from every untrusted caller).
//     No admission was needed for this port and none is made.
//   - agents.notifyWhen / close / orphans / reparent are in NEITHER scoped
//     tier's exact-name allowlist, so they are operator-only by construction —
//     the same standing they have with the desktop as provider. Nothing here
//     widens a tier; TestFleetVerbTiersAreUnchangedByThisPort pins that.
//
// WHAT THE STORE CAN AND CANNOT DO HERE. The desktop's store is authoritative:
// it owns row lifetime, eviction timers and its own tombstones. The brain's is
// a projection of claudemon's session list, and claudemon never deletes a row —
// it stops and archives them. That difference is load-bearing twice over, and
// each place says so: agents.close has to forget the row locally because the
// daemon keeps it, and agents.orphans needs NO tombstone store at all, because
// a dead manager is still present with status "ended".

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

// ── agents.reportProgress ───────────────────────────────────────────────────

// Bounds on a worker's self-reports. TWIN: NOTE_MAX / MIN_INTERVAL_MS /
// MAX_REPORTS in progressReports.ts, pinned by TestProgressBoundsMatchTheDesktop.
//
// Every one refuses OUT LOUD rather than truncating or silently dropping: these
// arrive unsolicited at a manager whose doctrine is never to poll, and a worker
// that believes it reported and did not is exactly the failure the tool exists
// to prevent.
const (
	progressNoteMax     = 500
	progressMinInterval = 60 * time.Second
	progressMaxReports  = 20
)

// progressBudget is one worker's lifetime allowance. In-memory and per-process,
// like the desktop's: a watch or a budget is a within-session intention, and
// persisting it would make it the jobs system.
type progressBudget struct {
	count    int
	lastAt   time.Time
	lastNote string
}

// reportProgress delivers one worker's progress line to whatever dispatched it.
//
// THE RECIPIENT IS NEVER A PARAMETER. The caller names no session but the one
// it claims to BE, and the recipient is derived from that session's own
// parentSessionId — so the only pair this can ever connect is (a tracked
// session, whatever dispatched it). A caller with no parent is REFUSED rather
// than routed somewhere plausible.
//
// `callerSessionId` is not a caller value on the path an agent actually uses:
// the MCP facade stamps it from the per-request token record's `session:<id>`
// label, and the hub router deletes it from every untrusted caller's params
// (sanitizeReportProgressParams, internal/bus/rpc.go) on the local AND the
// federated dispatch path. A scoped or plugin token carries no session identity
// to stamp from and therefore lands on the no-identity refusal below.
func (r *registry) reportProgress(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		CallerSessionID string `json:"callerSessionId"`
		Note            string `json:"note"`
		NeedsDecision   bool   `json:"needsDecision"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	callerID := flattenLine(p.CallerSessionID)
	if callerID == "" {
		// Only reachable from a credential with no session identity. Say so
		// rather than picking a recipient — guessing here is exactly the
		// containment hole this tool is defined not to have.
		return nil, fmt.Errorf("report_progress: the host could not identify your session from your credential, " +
			"so it cannot tell who dispatched you. This tool is for agents workspacer spawned; use send_message if you are driving the fleet.")
	}
	note := flattenLine(p.Note)
	if note == "" {
		return nil, fmt.Errorf("report_progress requires a non-empty note")
	}
	if len(note) > progressNoteMax {
		return nil, fmt.Errorf("report_progress: note is %d characters; the limit is %d. This is a progress LINE, not a report — "+
			"say what changed for your manager's decision (a phase finished, the approach is wrong, the budget is running out) "+
			"and leave the detail for your final message, which the finish wake delivers in full.", len(note), progressNoteMax)
	}

	all := r.fleetSessions(ctx)
	me, ok := findFleetSession(all, callerID)
	if !ok {
		return nil, fmt.Errorf("report_progress: session %s is not a tracked session", callerID)
	}
	parentID := me.ParentSessionID
	if parentID == "" || parentID == callerID {
		return nil, fmt.Errorf("report_progress: you have no parent session — nothing dispatched you, so there is nobody to report to. " +
			"Tell the user directly in your reply instead.")
	}
	parent, ok := findFleetSession(all, parentID)
	if !ok || parent.ended() {
		return nil, fmt.Errorf("report_progress: your parent session (%s) has ended — there is nobody to receive this. "+
			"Carry on and put it in your final message.", parentID)
	}

	now := time.Now()
	r.progressMu.Lock()
	if r.progress == nil {
		r.progress = map[string]progressBudget{}
	}
	budget := r.progress[callerID]
	switch {
	case budget.count >= progressMaxReports:
		r.progressMu.Unlock()
		return nil, fmt.Errorf("report_progress: you have already sent %d progress updates, which is the limit for one session. "+
			"Stop reporting and finish the task — your final message reaches your manager in full.", progressMaxReports)
	case note == budget.lastNote:
		// A retry loop double-waking the manager with the same sentence is the
		// cheapest way to make this channel unreadable.
		r.progressMu.Unlock()
		return nil, fmt.Errorf("report_progress: that is the same note you just sent; it was NOT delivered again.")
	}
	if since := now.Sub(budget.lastAt); budget.count > 0 && since < progressMinInterval {
		r.progressMu.Unlock()
		return nil, fmt.Errorf("report_progress: you reported %ds ago; updates are limited to one per %ds. "+
			"This one was NOT delivered — carry on working and fold it into your next update.",
			int(since.Round(time.Second)/time.Second), int(progressMinInterval/time.Second))
	}
	// Charge the budget BEFORE delivery: a failed send still consumed the
	// manager's attention budget as far as the worker is concerned, and a worker
	// retrying a failing send in a loop must still hit the cap.
	r.progress[callerID] = progressBudget{count: budget.count + 1, lastAt: now, lastNote: note}
	r.progressMu.Unlock()

	text := buildFleetMessage(fleetProgressHeader, fleetProgressTail, []fleetEntry{{
		Label:         me.displayLabel(),
		SessionID:     callerID,
		Cwd:           me.Cwd,
		Note:          note,
		NeedsDecision: p.NeedsDecision,
	}})
	if err := r.deliverFleetWake(ctx, parentID, text); err != nil {
		return nil, err
	}
	return jsonResult(map[string]any{"deliveredTo": parentID})
}

// ── agents.notifyWhen ───────────────────────────────────────────────────────

// thresholdSweepInterval is how often armed watches are evaluated. TWIN:
// SWEEP_MS. Fast enough that a runaway spend is caught within a turn or two,
// slow enough to be free — and a sweep cannot be starved by a chatty session
// the way per-snapshot evaluation could.
const thresholdSweepInterval = 15 * time.Second

// maxWatchesPerWatcher is a cap, not a policy: an agent arming watches in a
// loop would otherwise turn a wake channel into a firehose. TWIN:
// MAX_WATCHES_PER_WATCHER.
const maxWatchesPerWatcher = 20

var contextHealthProviders = map[string]bool{"claude": true, "codex": true, "copilot": true}
var noContextWindowProviders = map[string]bool{"opencode": true, "pi": true}

// thresholdWatch is one armed, ONE-SHOT watch. A re-arming watch would be a
// poll with extra steps.
type thresholdWatch struct {
	ID               string          `json:"id"`
	SessionID        string          `json:"sessionId"`
	WatcherSessionID string          `json:"watcherSessionId"`
	Tokens           *float64        `json:"tokens,omitempty"`
	USD              *float64        `json:"usd,omitempty"`
	IdleSeconds      *float64        `json:"idleSeconds,omitempty"`
	ContextUsedPct   *float64        `json:"contextUsedPct,omitempty"`
	ArmedAt          int64           `json:"armedAt"`
	ContextProvider  string          `json:"contextProvider,omitempty"`
	ContextEpoch     *telemetryEpoch `json:"contextEpoch,omitempty"`
	State            string          `json:"state,omitempty"`
}

// notifyWhen arms a one-shot threshold watch. It STARTS NOTHING and changes no
// session: it records an intention to send a message later, whose body is
// composed entirely by the host from the provider's own snapshot fields.
//
// Every failure is a refusal rather than a quiet no-op, because each one
// otherwise produces a manager that believes it is being watched and is not.
func (r *registry) notifyWhen(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID       string   `json:"sessionId"`
		NotifySessionID string   `json:"notifySessionId"`
		Tokens          *float64 `json:"tokens"`
		USD             *float64 `json:"usd"`
		IdleSeconds     *float64 `json:"idleSeconds"`
		ContextUsedPct  *float64 `json:"contextUsedPct"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("agents.notifyWhen requires { sessionId }")
	}
	for name, v := range map[string]*float64{"tokens": p.Tokens, "usd": p.USD, "idleSeconds": p.IdleSeconds} {
		if v != nil && (*v <= 0 || math.IsNaN(*v) || math.IsInf(*v, 0)) {
			return nil, fmt.Errorf("agents.notifyWhen: %s must be a positive number, got %v", name, *v)
		}
	}
	if p.ContextUsedPct != nil {
		if math.IsNaN(*p.ContextUsedPct) || math.IsInf(*p.ContextUsedPct, 0) || *p.ContextUsedPct <= 0 || *p.ContextUsedPct > 100 {
			return nil, fmt.Errorf("agents.notifyWhen: contextUsedPct must be a finite number in (0, 100], got %v", *p.ContextUsedPct)
		}
		if p.Tokens != nil || p.USD != nil || p.IdleSeconds != nil {
			return nil, fmt.Errorf("agents.notifyWhen: contextUsedPct is a single-purpose health predicate and cannot be combined with tokens, usd, or idleSeconds")
		}
	}
	if p.Tokens == nil && p.USD == nil && p.IdleSeconds == nil && p.ContextUsedPct == nil {
		// An empty predicate is a watch that can never fire, which reads to the
		// caller as "armed" and is worse than a refusal.
		return nil, fmt.Errorf("agents.notifyWhen requires at least one threshold: tokens, usd, idleSeconds, or contextUsedPct")
	}

	all := r.fleetSessions(ctx)
	target, ok := findFleetSession(all, p.SessionID)
	if !ok {
		return nil, fmt.Errorf("agents.notifyWhen: no such session %s", p.SessionID)
	}
	if target.ended() {
		return nil, fmt.Errorf("agents.notifyWhen: session %s has already ended — nothing left to watch", p.SessionID)
	}
	if p.ContextUsedPct != nil {
		provider := providerIdentity(target.Provider)
		if noContextWindowProviders[provider] {
			return nil, fmt.Errorf("agents.notifyWhen: contextUsedPct is unavailable for provider %s: it cannot emit a runtime context window", provider)
		}
		if provider != "" && !contextHealthProviders[provider] && target.contextHealth(time.Now()) == nil {
			// Future providers remain extensible when they prove the contract with
			// a correlated sample. Otherwise waiting would leak a permanent slot.
			return nil, fmt.Errorf("agents.notifyWhen: contextUsedPct cannot wait for unknown provider %s without a fresh runtime context sample", provider)
		}
	}
	// Default the recipient to the target's PARENT — the manager that dispatched
	// it is who wants to know, and it is the same routing the worker-finished
	// wake already uses. An explicit notifySessionId wins.
	watcherID := p.NotifySessionID
	if watcherID == "" {
		watcherID = target.ParentSessionID
	}
	if watcherID == "" {
		return nil, fmt.Errorf("agents.notifyWhen: no notifySessionId and the target has no parent session — " +
			"pass your own session id as notifySessionId")
	}
	if watcher, ok := findFleetSession(all, watcherID); !ok || watcher.ended() {
		return nil, fmt.Errorf("agents.notifyWhen: notifySessionId %s is not a live session — "+
			"a watch with no recipient would fire into nothing", watcherID)
	}

	r.watchMu.Lock()
	mine := 0
	for _, w := range r.watches {
		if w.WatcherSessionID == watcherID {
			mine++
		}
	}
	if mine >= maxWatchesPerWatcher {
		r.watchMu.Unlock()
		return nil, fmt.Errorf("agents.notifyWhen: %s already has %d armed watches (max %d) — "+
			"let some fire, or stop arming in a loop", watcherID, mine, maxWatchesPerWatcher)
	}
	if r.watches == nil {
		r.watches = map[string]*thresholdWatch{}
	}
	r.watchSeq++
	now := time.Now()
	w := &thresholdWatch{
		ID:               "w" + strconv.Itoa(r.watchSeq),
		SessionID:        p.SessionID,
		WatcherSessionID: watcherID,
		Tokens:           p.Tokens,
		USD:              p.USD,
		IdleSeconds:      p.IdleSeconds,
		ContextUsedPct:   p.ContextUsedPct,
		ArmedAt:          now.UnixMilli(),
	}
	if p.ContextUsedPct != nil {
		w.ContextProvider = providerIdentity(target.Provider)
		if health := target.contextHealth(now); health != nil {
			w.ContextProvider = health.Provider
			w.ContextEpoch = &health.Epoch
			if health.UsedPct >= *p.ContextUsedPct {
				w.State = "alreadySatisfied"
			} else {
				w.State = "armed"
			}
		} else {
			w.State = "waitingForTelemetry"
		}
	}
	r.watches[w.ID] = w
	r.watchMu.Unlock()
	return jsonResult(w)
}

// crossedBy renders the predicate this session has crossed, or "".
//
// Checked tokens → usd → idle so a session crossing two reports the one the
// caller most likely meant (spend before staleness), and ONE line, not three:
// the watch is one-shot, so one crossing ends it.
// TWIN: crossedBy in thresholdWatch.ts.
func crossedBy(w *thresholdWatch, s fleetSession, now time.Time) string {
	if w.ContextUsedPct != nil {
		return contextWatchCrossing(w, s, now)
	}
	if w.Tokens != nil {
		if tokens := s.tokens(); tokens >= *w.Tokens {
			return "tokens " + groupThousands(tokens) + " ≥ " + groupThousands(*w.Tokens)
		}
	}
	if w.USD != nil {
		if usd := s.costUSD(); usd >= *w.USD {
			return fmt.Sprintf("cost $%.2f ≥ $%.2f", usd, *w.USD)
		}
	}
	if w.IdleSeconds != nil {
		// "A worker that stopped without finishing" is this predicate's whole
		// purpose — and the most damaging way a worker stops is the one that
		// never reports idle. A wedged session reports `streaming` forever, so
		// gating on ambientState == "idle" would make the watch structurally
		// blind to the exact failure it was armed for.
		//
		// lastActivity is what makes broadening it safe: it moves on real
		// conversation deltas and ambient transitions and deliberately NOT on
		// statusline ticks, so "nothing has arrived in N seconds" is a fact about
		// output whatever the session claims to be doing.
		last := now
		if s.LastActivity > 0 {
			last = time.UnixMilli(s.LastActivity)
		}
		since := now.Sub(last)
		if since >= time.Duration(*w.IdleSeconds)*time.Second {
			secs := int(since.Round(time.Second) / time.Second)
			// The two cases read differently because they ARE different: a
			// session sitting at a prompt is done; one still claiming to work has
			// stalled. Naming the claimed state keeps the wake honest — a
			// genuinely long single tool call can trip this too.
			if s.AmbientState == "" || s.AmbientState == "idle" {
				return fmt.Sprintf("idle for %ds ≥ %ds", secs, int(*w.IdleSeconds))
			}
			return fmt.Sprintf("no activity for %ds ≥ %ds (still reports %s)", secs, int(*w.IdleSeconds), s.AmbientState)
		}
	}
	return ""
}

func normalizedProvider(provider string) string {
	provider = providerIdentity(provider)
	if provider == "" {
		return "unknown"
	}
	return provider
}

// providerIdentity normalizes a real provider while preserving absence. Empty
// hook-adopted rows are not an "unknown provider" refusal: a context watch may
// wait and bind once correlated telemetry names its owner.
func providerIdentity(provider string) string {
	return strings.ToLower(strings.TrimSpace(provider))
}

// Canonical desktop/Hub wake percentage: bounded and exactly one decimal.
func formatContextPct(v float64) string {
	if math.IsNaN(v) || math.IsInf(v, 0) {
		v = 0
	}
	v = math.Max(0, math.Min(100, v))
	// Decimal half-ties are rounded upward on the bounded non-negative domain,
	// matching the desktop's explicit Math.round implementation.
	v = math.Round(v*10) / 10
	return strconv.FormatFloat(v, 'f', 1, 64)
}

// contextWatchCrossing is the single authoritative context predicate used by
// production sweeps and direct tests. Ownership invalidation is part of the
// predicate; it cannot be bypassed by an epoch-blind numeric-only branch.
func contextWatchCrossing(w *thresholdWatch, s fleetSession, now time.Time) string {
	targetProvider := normalizedProvider(s.Provider)
	targetIdentity := providerIdentity(s.Provider)
	if w.ContextProvider != "" && targetIdentity != "" && targetIdentity != providerIdentity(w.ContextProvider) {
		return fmt.Sprintf("monitoring invalidated: contextUsedPct %s%% watch crossed a provider/session boundary (%s → %s); re-arm after a confirmed sample", formatContextPct(*w.ContextUsedPct), w.ContextProvider, targetProvider)
	}
	if targetIdentity != "" && noContextWindowProviders[targetIdentity] {
		return fmt.Sprintf("monitoring invalidated: contextUsedPct %s%% is unavailable for provider %s; re-arm only on a provider with runtime context telemetry", formatContextPct(*w.ContextUsedPct), targetProvider)
	}
	health := s.contextHealth(now)
	if health == nil {
		return ""
	}
	if w.ContextEpoch != nil && *w.ContextEpoch != health.Epoch {
		return fmt.Sprintf("monitoring invalidated: contextUsedPct %s%% watch crossed telemetry epoch %s → %s; re-arm after the confirmed %s sample", formatContextPct(*w.ContextUsedPct), *w.ContextEpoch, health.Epoch, health.Provider)
	}
	if w.ContextProvider == "" {
		w.ContextProvider = health.Provider
	}
	if w.ContextEpoch == nil {
		epoch := health.Epoch
		w.ContextEpoch = &epoch
	}
	if health.UsedPct >= *w.ContextUsedPct {
		return contextCrossing(*w.ContextUsedPct, health)
	}
	return ""
}

func contextCrossing(threshold float64, h *contextHealthReading) string {
	return fmt.Sprintf(
		"contextUsedPct active context %s%% ≥ %s%% (%s / %s tokens; runtime-confirmed by %s; observed %s; epoch %s)",
		formatContextPct(h.UsedPct), formatContextPct(threshold), groupThousands(h.UsedTokens),
		groupThousands(h.WindowTokens), h.Provider, h.ObservedAt, h.Epoch,
	)
}

// groupThousands renders a token count the way toLocaleString('en-US') does —
// the wake text is compared by humans against numbers the desktop produced.
func groupThousands(v float64) string {
	digits := strconv.FormatInt(int64(v), 10)
	neg := ""
	if digits != "" && digits[0] == '-' {
		neg, digits = "-", digits[1:]
	}
	var out []byte
	for i, c := range []byte(digits) {
		if i > 0 && (len(digits)-i)%3 == 0 {
			out = append(out, ',')
		}
		out = append(out, c)
	}
	return neg + string(out)
}

// sweepThresholds evaluates every armed watch once.
//
// A watch whose TARGET has ended is dropped, not fired: the finish wake already
// told the manager, and a second "and by the way it cost $9" after the fact is
// noise. A watch whose WATCHER has ended is dropped too — nobody is listening.
func (r *registry) sweepThresholds(ctx context.Context, now time.Time) {
	r.watchMu.Lock()
	if len(r.watches) == 0 {
		r.watchMu.Unlock()
		return
	}
	all := r.fleetSessions(ctx)
	byID := map[string]fleetSession{}
	for _, s := range all {
		byID[s.SessionID] = s
	}
	fired := map[string][]fleetEntry{}
	for id, w := range r.watches {
		target, hasTarget := byID[w.SessionID]
		watcher, hasWatcher := byID[w.WatcherSessionID]
		if !hasTarget || target.ended() || !hasWatcher || watcher.ended() {
			delete(r.watches, id)
			continue
		}
		crossed := crossedBy(w, target, now)
		if crossed == "" {
			continue
		}
		delete(r.watches, id) // one-shot: gone before delivery, never twice
		fired[w.WatcherSessionID] = append(fired[w.WatcherSessionID], fleetEntry{
			Label:     target.displayLabel(),
			SessionID: target.SessionID,
			Cwd:       target.Cwd,
			Crossed:   crossed,
		})
	}
	r.watchMu.Unlock()

	// One wake per WATCHER carrying every threshold that crossed this sweep —
	// the same coalescing every other fleet wake does. Ordered by session id so
	// a multi-entry wake is deterministic rather than map-ordered.
	for watcherID, entries := range fired {
		sort.Slice(entries, func(i, j int) bool { return entries[i].SessionID < entries[j].SessionID })
		text := buildFleetMessage(fleetThresholdHeader, fleetThresholdTail, entries)
		// Best-effort, exactly like the desktop's: the watcher may have ended
		// between the check above and the send.
		_ = r.deliverFleetWake(ctx, watcherID, text)
	}
}

// runThresholdSweeps drives sweepThresholds until ctx ends. Started from main
// in full scope only — catalog scope has no live store to sweep.
func (r *registry) runThresholdSweeps(ctx context.Context) {
	ticker := time.NewTicker(thresholdSweepInterval)
	defer ticker.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case now := <-ticker.C:
			r.sweepThresholds(ctx, now)
		}
	}
}

// ── agents.close ────────────────────────────────────────────────────────────

// closeAgent dismisses a finished session: it forgets the row and, for a row
// that had not already ended, tears the daemon side down too.
//
// IT CANNOT BE AIMED AT A WORKING SESSION. The refusal is checked BEFORE any
// teardown, so a refused call leaves the worker exactly as it was — hiding a
// running agent from list_agents while it kept spending is the only outcome
// worse than the lingering row this replaces.
//
// The row is forgotten LOCALLY because claudemon does not forget it: the daemon
// keeps a stopped session as a resumable row on purpose, so a close that only
// signalled would leave list_agents unchanged and the verb would do nothing
// visible at all.
func (r *registry) closeAgent(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		SessionID string `json:"sessionId"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.SessionID == "" {
		return nil, fmt.Errorf("agents.close requires { sessionId }")
	}
	if r.store == nil {
		return nil, fmt.Errorf("agents.close: this brain holds no live session store (catalog scope) — there is no row to close")
	}
	before, known := findFleetSession(r.fleetSessions(ctx), p.SessionID)
	if !known {
		return jsonResult(map[string]any{
			"ok": true, "removed": false, "wasLive": false, "daemon": "already-ended",
			"note": "No such session — it had already been forgotten. Nothing to do.",
		})
	}
	working := !before.ended() && (before.AmbientState == "thinking" || before.AmbientState == "streaming" || before.AmbientState == "background")
	if working {
		return nil, fmt.Errorf("close_session: %s is still working (%s). Dismissing it would hide a running agent from "+
			"list_agents while it kept spending — stop it first (signal SIGTERM), then close it.", p.SessionID, before.AmbientState)
	}
	wasLive := !before.ended()
	r.store.remove(p.SessionID)
	// The wake watcher keeps two per-session entries (a dedup signature and the
	// remembered ambient state). The row is gone, so drop them with it rather
	// than retaining one per session for the process lifetime — the same concern
	// supervisorNudge.forgetWorker answers on the desktop.
	if r.fin != nil {
		r.fin.forgetWorker(p.SessionID)
	}

	// Tear the daemon side down too, but only for a row that had not already
	// ended: a dismissal that left a live-but-idle wrapper attached would be a
	// lie, and re-SIGTERMing an ended session is the pointless call whose 404
	// this verb exists to replace. Best-effort — the row is gone either way.
	daemon := "already-ended"
	if wasLive {
		if err := r.cm.closeSession(ctx, p.SessionID); err != nil {
			daemon = "failed"
		} else {
			daemon = "stopped"
		}
	}
	out := map[string]any{
		"ok": true, "removed": true, "wasLive": wasLive, "daemon": daemon,
		"note": "The session is gone from list_agents. Its desktop PANE, if the user has one open, is theirs to close.",
	}
	if before.Label != "" {
		out["label"] = before.Label
	}
	return jsonResult(out)
}

// ── agents.orphans / agents.reparent ────────────────────────────────────────

// orphanCandidate is one dead parent that still has live children.
type orphanCandidate struct {
	SessionID string `json:"sessionId"`
	Label     string `json:"label,omitempty"`
	Cwd       string `json:"cwd,omitempty"`
	// ConfirmedManager is true when the dead parent's own row said it was a
	// supervisor. A dangling parent id whose row is GONE cannot be confirmed
	// either way, and reporting it as "not a manager" would be a claim the data
	// does not support.
	ConfirmedManager bool `json:"confirmedManager"`
	// EndedAt is when the row was last active, ms since epoch — 0 when the row
	// itself is gone.
	EndedAt int64 `json:"endedAt,omitempty"`
	// Children are the live sessions still pointing at it.
	Children []string `json:"children"`
}

// orphans reports the DEAD parents that still have live children — the
// `fromSessionId` agents.reparent needs when the manager being replaced crashed
// and wrote no handoff file to read one off.
//
// IT REPORTS, IT NEVER ADOPTS. Folding this into a no-argument "adopt whatever
// is orphaned" mode is refused for the reason the desktop refuses it:
// confirmedManager narrows the candidates but cannot say which manager was
// YOURS, and a wrong guess silently re-points a live worker's wakes into a
// conversation that never dispatched it.
//
// NO TOMBSTONE STORE IS NEEDED HERE, and that is a real difference from the
// desktop rather than a shortcut. The desktop's store evicts a row ~30s after
// SessionEnd and therefore had to keep a tombstone to answer this at all; the
// brain's store projects claudemon, which never deletes a session — it stops
// and archives it — so the dead manager is still present with status "ended".
// The one case that reduces to a bare id is a parent claudemon has genuinely
// forgotten (a wiped state dir): reported with no label and confirmedManager
// false, because that is exactly what is known.
func (r *registry) orphans(ctx context.Context, _ json.RawMessage) (json.RawMessage, error) {
	all := r.fleetSessions(ctx)
	byID := map[string]fleetSession{}
	for _, s := range all {
		byID[s.SessionID] = s
	}
	children := map[string][]string{}
	for _, s := range all {
		if s.ParentSessionID == "" || s.ParentSessionID == s.SessionID || s.ended() {
			continue
		}
		parent, known := byID[s.ParentSessionID]
		if known && !parent.ended() {
			continue // the parent is alive; nothing is orphaned
		}
		children[s.ParentSessionID] = append(children[s.ParentSessionID], s.SessionID)
	}

	candidates := make([]orphanCandidate, 0, len(children))
	for parentID, kids := range children {
		sort.Strings(kids)
		c := orphanCandidate{SessionID: parentID, Children: kids}
		if parent, known := byID[parentID]; known {
			c.Label = parent.Label
			c.Cwd = parent.Cwd
			c.ConfirmedManager = parent.IsWakeTarget
			c.EndedAt = parent.LastActivity
		}
		candidates = append(candidates, c)
	}
	// Most children first, then by id: the top row is usually the obvious one,
	// and ties are deterministic rather than map-ordered.
	sort.Slice(candidates, func(i, j int) bool {
		if len(candidates[i].Children) != len(candidates[j].Children) {
			return len(candidates[i].Children) > len(candidates[j].Children)
		}
		return candidates[i].SessionID < candidates[j].SessionID
	})

	confirmed := 0
	for _, c := range candidates {
		if c.ConfirmedManager {
			confirmed++
		}
	}
	// "None" is a real and common answer (the predecessor finished its
	// dispatches, or handed over cleanly) and must not read as a failure.
	note := "Nothing is orphaned here: every live agent either has a live parent or was never dispatched by one."
	if len(candidates) > 0 {
		note = fmt.Sprintf("%d dead parent(s) still have live children; %d are confirmed managers. "+
			"Pick the one you are replacing — match its label/cwd against what you were told to take over — and pass "+
			"its sessionId as fromSessionId to adopt_workers. Adopting the wrong group re-points another manager's "+
			"workers onto you, so do not guess between two candidates: read a worker of each first.", len(candidates), confirmed)
	}
	return jsonResult(map[string]any{"candidates": candidates, "note": note})
}

// reparent re-points a retiring manager's dispatches at its successor.
//
// THE CALLER IS THE SUCCESSOR and it names itself, deliberately: the verified
// replacement recipe destroys the outgoing manager before the new one exists,
// so there is no moment at which the outgoing session could name its own
// successor — but the successor boots knowing both ids. Doing it automatically
// on spawn was rejected because the host would have to GUESS which dead manager
// a fresh one replaces, and a wrong guess silently re-points a live worker's
// wakes into a conversation that never dispatched it.
//
// THE REFUSALS ARE THE CONFINEMENT. This verb acts on OTHER sessions, so what
// bounds it is not a path but the destination: a successor that is unknown,
// ended, or not a manager cannot receive a wake at all, and re-pointing workers
// at one is worse than the orphaning it fixes. Those three are checked BEFORE
// anything moves, so a refusal leaves every parent pointer exactly as it was.
func (r *registry) reparent(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var p struct {
		FromSessionID string `json:"fromSessionId"`
		ToSessionID   string `json:"toSessionId"`
	}
	if err := unmarshal(raw, &p); err != nil {
		return nil, err
	}
	if p.FromSessionID == "" || p.ToSessionID == "" {
		return nil, fmt.Errorf("agents.reparent requires { fromSessionId, toSessionId }")
	}
	if p.FromSessionID == p.ToSessionID {
		return nil, fmt.Errorf("agents.reparent: fromSessionId and toSessionId are the same session (%s) — nothing to move", p.FromSessionID)
	}
	if r.meta == nil {
		return nil, fmt.Errorf("agents.reparent: this brain holds no spawn metadata (catalog scope) — there is no parent link to move")
	}
	all := r.fleetSessions(ctx)
	to, known := findFleetSession(all, p.ToSessionID)
	if !known {
		return nil, fmt.Errorf("agents.reparent: no such session %s — a wake cannot reach a session this host does not know", p.ToSessionID)
	}
	if to.ended() {
		return nil, fmt.Errorf("agents.reparent: session %s has ended — re-pointing live workers at it would silence them", p.ToSessionID)
	}
	if !to.IsWakeTarget {
		return nil, fmt.Errorf("agents.reparent: session %s is not a manager (isWakeTarget is not set) — "+
			"fleet wakes are only delivered to a supervisor, so this would silence every worker moved onto it", p.ToSessionID)
	}

	moved, pending := r.meta.reparentChildren(p.FromSessionID, p.ToSessionID, func(id string) bool {
		s, ok := findFleetSession(all, id)
		return ok && !s.ended()
	})
	// The store's rows carry the OLD parent until the next snapshot lands, so
	// re-enrich them now: a manager that adopts and immediately lists its fleet
	// must see the move it just made, not the state before it.
	r.restampParents(moved)

	count := len(moved) + len(pending)
	// Say which way it went, because "0 moved" is a real and useful answer — the
	// predecessor had nothing in flight — and must not read as a failure.
	note := fmt.Sprintf("Nothing was still parented to %s — it had no dispatch left in flight. "+
		"Any result you are owed is on disk or in its transcript.", p.FromSessionID)
	if count > 0 {
		note = fmt.Sprintf("%d dispatch(es) now report to %s: their finished and progress wakes arrive here, not at %s.",
			count, p.ToSessionID, p.FromSessionID)
	}
	return jsonResult(map[string]any{"moved": moved, "pending": pending, "note": note})
}

// restampParents re-runs the store's enrichment over the named rows so a
// reparent is visible on the very next agents.list rather than at the next
// claudemon event. Deliberately silent (no onChange publish): the parent link
// is not a state change the fleet needs pushed, and re-publishing a row the
// visibility rule might hide is a separate decision.
func (r *registry) restampParents(ids []string) {
	if r.store == nil {
		return
	}
	for _, id := range ids {
		r.store.restamp(id)
	}
}

// registry state for the fleet verbs. Kept beside the handlers rather than in
// the struct literal so the locking discipline reads next to its users:
// progressMu guards the per-worker budgets, watchMu the armed watches.
type fleetState struct {
	progressMu sync.Mutex
	progress   map[string]progressBudget

	watchMu  sync.Mutex
	watches  map[string]*thresholdWatch
	watchSeq int
}
