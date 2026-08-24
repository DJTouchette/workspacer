// Package quiescence answers one question, server-side and read-only: is this
// machine's fleet genuinely at rest?
//
// It exists so an operator can automate "power the box down while nothing is
// happening" without reimplementing the safety logic in an unversioned shell
// script. The predicate lives in Go, next to the state it reads, and is pinned
// by table tests; the DECISION — what to do with the answer, on whatever
// platform this is running — stays entirely with the operator's own script.
// Nothing here knows or asserts anything about any hosting provider.
//
// THE GOVERNING RULE IS REFUSE WHEN UNSURE. A missed opportunity costs a
// fraction of a cent. A wrong answer destroys somebody's in-flight work. Every
// ambiguous case therefore resolves to "not quiescent": a session in a mode
// this package does not recognise blocks, a row it cannot parse blocks, a
// session provider that does not answer blocks, a peer it cannot reach blocks,
// and a sample gap it cannot account for restarts the clock. The savings that
// costs are real and named in the package documentation; they are the price of
// the guarantee.
//
// The answer is never an instant reading. Blockers must be absent CONTINUOUSLY
// for a dwell (see [Tunables.Dwell]) before quiescent goes true, which is what
// makes the one genuine gap in workspacer's session state tolerable: for a
// managed or stream session nothing periodically re-asserts liveness, so
// "working" is inferred from the absence of a turn-end rather than from a
// heartbeat. That gap fails in the safe direction — a wedged agent stays
// `responding` forever and pins the machine awake, costing money rather than
// work — and a dwell measured in minutes covers the rest: an agent that is
// genuinely thinking emits something inside that window in any realistic case.
package quiescence

import (
	"fmt"
	"sort"
	"sync"
	"time"
)

// ── Tunables ────────────────────────────────────────────────────────────────
//
// Named and gathered here rather than spelled as literals at the sites that
// use them, because the first thing anyone does after watching this run against
// their own fleet for a week is relax one of them. The defaults are set for
// certainty, not for yield.

// Tunables are the predicate's timing knobs.
type Tunables struct {
	// Dwell is how long every blocker must stay absent before quiescent goes
	// true. Below ~10 minutes the managed-session liveness gap starts to
	// matter (see the package doc); above ~20 the signal stops firing on
	// ordinary overnight quiet.
	Dwell time.Duration
	// ClientIdleWindow is how long a connected bus client must have been
	// SILENT before it stops counting as somebody using the machine.
	//
	// Connection presence alone is the wrong test and would make this signal
	// almost useless: a phone with the mobile client open in a background tab
	// holds a socket and reconnects forever. What it does not do is send
	// anything. So a client is "active" while it is still calling capabilities
	// or publishing, and goes quiet on its own after this long.
	ClientIdleWindow time.Duration
	// JobLookahead is how far ahead a scheduled job counts as due. A machine
	// that powers down ninety seconds before a nightly review fires has not
	// saved anything; it has just made the review late.
	JobLookahead time.Duration
	// MaxSampleGap is the longest interval between two observations that can
	// still be treated as continuous. A longer gap means nobody was watching,
	// and an unwatched stretch cannot be counted toward the dwell — so the
	// clock restarts. It also bounds staleness on the read side: an answer
	// older than this is refused rather than served.
	MaxSampleGap time.Duration
}

// Default tunable values. Deliberately conservative; see [Tunables].
const (
	DefaultDwell            = 12 * time.Minute
	DefaultClientIdleWindow = 10 * time.Minute
	DefaultJobLookahead     = 15 * time.Minute
	DefaultMaxSampleGap     = 3 * time.Minute
	// DefaultSampleInterval is how often the hub takes a reading. It is not
	// part of [Tunables] because it governs the sampler rather than the
	// predicate, but it must stay comfortably under MaxSampleGap or an
	// ordinary slow tick would reset the dwell.
	DefaultSampleInterval = 30 * time.Second
)

// DefaultTunables returns the shipped settings.
func DefaultTunables() Tunables {
	return Tunables{
		Dwell:            DefaultDwell,
		ClientIdleWindow: DefaultClientIdleWindow,
		JobLookahead:     DefaultJobLookahead,
		MaxSampleGap:     DefaultMaxSampleGap,
	}
}

// withDefaults fills any zero field, so a caller that sets one knob does not
// silently disable the others.
func (t Tunables) withDefaults() Tunables {
	d := DefaultTunables()
	if t.Dwell <= 0 {
		t.Dwell = d.Dwell
	}
	if t.ClientIdleWindow <= 0 {
		t.ClientIdleWindow = d.ClientIdleWindow
	}
	if t.JobLookahead <= 0 {
		t.JobLookahead = d.JobLookahead
	}
	if t.MaxSampleGap <= 0 {
		t.MaxSampleGap = d.MaxSampleGap
	}
	return t
}

// ── The answer ──────────────────────────────────────────────────────────────

// Blocker kinds. Closed vocabulary: a caller may switch on these, and a
// blocker whose kind it does not recognise still reads as a reason to stay up.
const (
	KindSessionWorking    = "session-working"
	KindSessionUnknown    = "session-unknown"
	KindSessionUnreadable = "session-unreadable"
	KindBackgroundTasks   = "background-tasks"
	KindPendingApproval   = "pending-approval"
	KindPendingQuestion   = "pending-question"
	KindFleetUnreadable   = "fleet-unreadable"
	KindClientActive      = "client-active"
	KindJobDueSoon        = "job-due-soon"
	KindJobRunning        = "job-running"
	KindPeerUnreachable   = "peer-unreachable"
	KindDwell             = "dwell"
	KindStaleSample       = "stale-sample"
)

// Blocker is one named reason the fleet is not at rest. A bare `false` is not
// something anyone can operate on: the whole point of this list is that when
// the answer is no, the caller can see exactly what is holding the machine up
// and go and deal with it.
type Blocker struct {
	Kind string `json:"kind"`
	// ID names the thing, when the thing has a name: a session id (prefixed
	// `hub:<peer>/` for a federated row), a job id, a peer name.
	ID string `json:"id,omitempty"`
	// Detail is one human sentence. It is the field an operator actually
	// reads, so it says what is true rather than restating the kind.
	Detail string `json:"detail"`
}

// Result is what the bus method answers.
type Result struct {
	Quiescent bool `json:"quiescent"`
	// Since is the unix-millisecond instant the current unbroken calm began —
	// NOT the instant the dwell elapsed. Null whenever Quiescent is false.
	Since *int64 `json:"since"`
	// Blockers is empty exactly when Quiescent is true.
	Blockers []Blocker `json:"blockers"`
	// DwellSeconds and CalmSeconds let a caller see how close it is: how long
	// the calm must hold, and how long it has held. CalmSeconds is 0 whenever
	// anything is blocking.
	DwellSeconds int64 `json:"dwellSeconds"`
	CalmSeconds  int64 `json:"calmSeconds"`
}

// ── What the predicate reads ────────────────────────────────────────────────

// Session is one session row, already reduced to the fields the predicate
// cares about. [ParseSessions] builds these from whatever a session provider
// answered; the split keeps the predicate pure and table-testable.
type Session struct {
	ID string
	// Peer is empty for a local row, or the federated peer's name.
	Peer string
	// Mode is claudemon's own vocabulary (input / responding / approval /
	// question / stopped / unknown) when the row carried it. It is preferred
	// over Ambient because it is the authoritative state machine; Ambient is
	// an overlay derived from it.
	Mode string
	// Ambient is the desktop vocabulary (idle / thinking / streaming /
	// waiting_input / waiting_approval / background), for a row from a
	// provider that speaks only that.
	Ambient string
	// State is a one-line rendering of whichever of the two was used, for the
	// blocker detail.
	State           string
	BackgroundTasks int
	PendingApproval bool
	PendingQuestion bool
	// Ended marks a session that has stopped. Ended sessions block nothing.
	Ended bool
	// Unreadable marks a row whose state could not be determined at all. It
	// blocks, because "I could not tell" is not "nothing is happening".
	Unreadable string
}

// Ref is how a session is named in a blocker: bare locally, peer-qualified for
// a federated row, matching the `hub:<peer>/…` form the bus already uses.
func (s Session) Ref() string {
	if s.Peer == "" {
		return s.ID
	}
	return "hub:" + s.Peer + "/" + s.ID
}

// Client is one live bus connection that belongs to a USER of this machine —
// a phone, a browser, a terminal client, the MCP facade an agent talks through.
// Infrastructure connections (a capability provider, a plugin sidecar, the
// hub's own loopback client, and the caller asking this very question) are not
// clients and never appear here: counting them would mean the machine could
// never be at rest while it was running normally, which is not conservatism,
// just a broken signal.
type Client struct {
	// Label describes the connection in the words an operator would use
	// ("operator token", "plugin shiplight"), never a credential.
	Label string
	// LastActive is when this connection last CALLED or PUBLISHED something,
	// or when it connected, whichever is later.
	LastActive time.Time
}

// Job is one hub job, reduced to what the predicate needs.
type Job struct {
	ID   string
	Name string
	// ActionKind is "spawn" | "call" | "shell".
	ActionKind string
	// NextRun is zero when the job is not scheduled (disabled, manual, or an
	// unapproved proposal).
	NextRun time.Time
	Running bool
}

// Peer is a federated peer hub and whether its state could be established.
// Err non-empty means it could not, which blocks: an unreachable peer is not
// a quiet one, it is an unknown one.
type Peer struct {
	Name string
	Err  string
}

// Inputs is one complete reading of the machine.
type Inputs struct {
	Now time.Time
	// Sessions is every session row, local and federated.
	Sessions []Session
	// SessionsErr is set when the fleet could not be read at all — no
	// provider answered, or the answer did not parse. It blocks outright: a
	// missing answer is not an empty fleet.
	SessionsErr error
	Clients     []Client
	Jobs        []Job
	Peers       []Peer
}

// ── The predicate ───────────────────────────────────────────────────────────

// workingModes are claudemon modes that mean the session is producing a turn.
var workingModes = map[string]bool{"responding": true}

// restingModes are claudemon modes that mean the session is doing nothing.
// Everything NOT in this map and not in workingModes — `unknown` above all —
// blocks, because the mode vocabulary is the daemon's and a value this package
// has never heard of is not a value it may read as calm.
var restingModes = map[string]bool{"input": true}

// blockedModes are the two "waiting on a human" modes, kept separate so the
// blocker names which one.
var blockedModes = map[string]string{
	"approval": KindPendingApproval,
	"question": KindPendingQuestion,
}

// workingAmbient are the desktop ambient states that mean work is in flight.
var workingAmbient = map[string]bool{
	"thinking": true, "streaming": true, "background": true,
}

// restingAmbient is the ONE ambient state that means at rest.
var restingAmbient = map[string]bool{"idle": true}

var blockedAmbient = map[string]string{
	"waiting_approval": KindPendingApproval,
	"waiting_input":    KindPendingQuestion,
}

// Evaluate applies the predicate to one reading and returns every reason the
// fleet is not at rest. An empty result means calm AT THIS INSTANT; the dwell
// is [Monitor]'s job.
func Evaluate(in Inputs, t Tunables) []Blocker {
	t = t.withDefaults()
	var out []Blocker

	if in.SessionsErr != nil {
		out = append(out, Blocker{
			Kind:   KindFleetUnreadable,
			Detail: "could not read the fleet: " + in.SessionsErr.Error() + ". No answer is not an empty fleet, so this blocks",
		})
	}
	out = append(out, sessionBlockers(in.Sessions)...)
	out = append(out, clientBlockers(in.Now, in.Clients, t)...)
	out = append(out, jobBlockers(in.Now, in.Jobs, t)...)
	for _, p := range in.Peers {
		if p.Err != "" {
			out = append(out, Blocker{
				Kind:   KindPeerUnreachable,
				ID:     p.Name,
				Detail: "peer " + p.Name + " could not be checked: " + p.Err + ". An unreachable peer is an unknown one, not a quiet one",
			})
		}
	}
	return out
}

func sessionBlockers(sessions []Session) []Blocker {
	var out []Blocker
	for _, s := range sessions {
		if s.Ended {
			continue
		}
		if s.Unreadable != "" {
			out = append(out, Blocker{
				Kind:   KindSessionUnreadable,
				ID:     s.Ref(),
				Detail: "session row could not be read (" + s.Unreadable + "), so nothing here can say it is finished",
			})
			continue
		}
		out = append(out, stateBlocker(s)...)
		if s.BackgroundTasks > 0 {
			out = append(out, Blocker{
				Kind: KindBackgroundTasks,
				ID:   s.Ref(),
				Detail: fmt.Sprintf(
					"%d background task(s) still running. The session mode deliberately does not go busy for these — a dev server, a watcher, an agent-authored poll loop — so the count is the only signal they exist",
					s.BackgroundTasks),
			})
		}
		if s.PendingApproval {
			out = append(out, Blocker{
				Kind:   KindPendingApproval,
				ID:     s.Ref(),
				Detail: "waiting on a permission decision. Nothing is in flight, so this is the safest state there is to interrupt — it blocks in v1 because a sleeping machine sends no push, and it is the first thing to relax once you have measured what it costs",
			})
		}
		if s.PendingQuestion {
			out = append(out, Blocker{
				Kind:   KindPendingQuestion,
				ID:     s.Ref(),
				Detail: "waiting on an answer to a question it asked. Same reasoning as a pending approval",
			})
		}
	}
	return out
}

// stateBlocker reads the session's own state machine. Mode wins over the
// ambient overlay: the overlay is derived FROM the mode and cannot express
// everything the mode can.
func stateBlocker(s Session) []Blocker {
	if s.Mode != "" {
		switch {
		case workingModes[s.Mode]:
			return []Blocker{{Kind: KindSessionWorking, ID: s.Ref(),
				Detail: "producing a turn (mode=" + s.Mode + ")"}}
		case restingModes[s.Mode]:
			return nil
		default:
			if kind, ok := blockedModes[s.Mode]; ok {
				// The pending slot reports these too; this arm catches a row
				// whose mode says blocked while the slot did not travel.
				return []Blocker{{Kind: kind, ID: s.Ref(),
					Detail: "blocked on a human (mode=" + s.Mode + ")"}}
			}
			return []Blocker{{Kind: KindSessionUnknown, ID: s.Ref(), Detail: unknownDetail(s.Mode)}}
		}
	}
	if s.Ambient != "" {
		switch {
		case workingAmbient[s.Ambient]:
			return []Blocker{{Kind: KindSessionWorking, ID: s.Ref(),
				Detail: "working (state=" + s.Ambient + ")"}}
		case restingAmbient[s.Ambient]:
			return nil
		default:
			if kind, ok := blockedAmbient[s.Ambient]; ok {
				return []Blocker{{Kind: kind, ID: s.Ref(),
					Detail: "blocked on a human (state=" + s.Ambient + ")"}}
			}
			return []Blocker{{Kind: KindSessionUnknown, ID: s.Ref(), Detail: unknownDetail(s.Ambient)}}
		}
	}
	return []Blocker{{Kind: KindSessionUnknown, ID: s.Ref(),
		Detail: unknownDetail("")}}
}

// unknownDetail says plainly what a session nobody has heard from might be,
// because two of the three possibilities are things an operator can act on and
// the third is a permanent property of terminals that is worth knowing about.
func unknownDetail(state string) string {
	shown := state
	if shown == "" {
		shown = "none reported"
	}
	return "state " + shown + ": either a session that is SPAWNING or resuming and has not reported yet, " +
		"or a TERMINAL. Nothing anywhere in workspacer tracks whether a terminal's shell is running a " +
		"long command, so a live terminal blocks on presence alone and will keep blocking until it is closed"
}

func clientBlockers(now time.Time, clients []Client, t Tunables) []Blocker {
	var out []Blocker
	for _, c := range clients {
		idle := now.Sub(c.LastActive)
		if idle >= t.ClientIdleWindow {
			continue
		}
		out = append(out, Blocker{
			Kind: KindClientActive,
			Detail: fmt.Sprintf("%s last did something %s ago; a client counts as in use until it has been silent for %s",
				c.Label, roundDur(idle), t.ClientIdleWindow),
		})
	}
	return out
}

// jobBlockers reports scheduled work that would be stranded.
//
// SHELL JOBS ARE DELIBERATELY NOT COUNTED, and this is a limitation rather
// than an oversight: the shell action is how an operator runs this check in
// the first place, so a poller that counted itself as due would report a
// blocker forever and the signal would never fire. The consequence is stated
// rather than hidden — a shell job that does real work is not protected by
// this blocker, and work that must be protected belongs behind a spawn or call
// action, or behind the operator's own check inside the script.
func jobBlockers(now time.Time, jobs []Job, t Tunables) []Blocker {
	var out []Blocker
	for _, j := range jobs {
		if j.ActionKind == "shell" {
			continue
		}
		name := j.Name
		if name == "" {
			name = j.ID
		}
		if j.Running {
			out = append(out, Blocker{Kind: KindJobRunning, ID: j.ID,
				Detail: "job " + name + " is running right now"})
			continue
		}
		if j.NextRun.IsZero() {
			continue
		}
		if until := j.NextRun.Sub(now); until <= t.JobLookahead {
			out = append(out, Blocker{Kind: KindJobDueSoon, ID: j.ID,
				Detail: fmt.Sprintf("job %s is due in %s (lookahead %s)", name, roundDur(until), t.JobLookahead)})
		}
	}
	return out
}

// roundDur renders a duration the way an operator reads one. Negative (already
// overdue, already in the past) renders as "0s" rather than as a minus sign.
func roundDur(d time.Duration) time.Duration {
	if d < 0 {
		return 0
	}
	if d < time.Minute {
		return d.Round(time.Second)
	}
	return d.Round(time.Minute)
}

// ── The dwell ───────────────────────────────────────────────────────────────

// Monitor turns a stream of readings into the answer, holding the one piece of
// state the predicate itself does not: how long the calm has lasted.
//
// It is sampled on a timer by the hub rather than computed on demand, because
// "held continuously" cannot be established from a single reading taken at the
// moment somebody asked. A caller that polls every five minutes and computed
// the dwell from its own polls would be asserting continuity across four
// minutes and fifty seconds it never looked at.
type Monitor struct {
	tun Tunables
	now func() time.Time

	mu         sync.Mutex
	calmSince  time.Time
	lastSample time.Time
	blockers   []Blocker
}

// NewMonitor builds a monitor. A zero Tunables field takes its default.
func NewMonitor(t Tunables) *Monitor {
	return &Monitor{tun: t.withDefaults(), now: time.Now}
}

// SetClock replaces the monitor's clock. Tests only.
func (m *Monitor) SetClock(fn func() time.Time) {
	m.mu.Lock()
	m.now = fn
	m.mu.Unlock()
}

// Tunables returns the settings in force.
func (m *Monitor) Tunables() Tunables { return m.tun }

// Observe folds one reading in and returns the answer as of that reading.
func (m *Monitor) Observe(in Inputs) Result {
	blockers := Evaluate(in, m.tun)
	m.mu.Lock()
	defer m.mu.Unlock()

	// A gap longer than MaxSampleGap is a stretch nobody watched, and an
	// unwatched stretch cannot count toward "held continuously". This is what
	// makes a paused, suspended or wedged hub restart the clock rather than
	// wake up believing the fleet was calm the whole time.
	if !m.lastSample.IsZero() && in.Now.Sub(m.lastSample) > m.tun.MaxSampleGap {
		m.calmSince = time.Time{}
	}
	m.lastSample = in.Now
	m.blockers = blockers

	if len(blockers) > 0 {
		m.calmSince = time.Time{}
		return m.resultLocked(in.Now)
	}
	if m.calmSince.IsZero() {
		m.calmSince = in.Now
	}
	return m.resultLocked(in.Now)
}

// Latest answers from the most recent reading, refusing an answer that has
// gone stale: if the sampler stopped, the last thing it saw is not evidence
// about now.
func (m *Monitor) Latest() Result {
	now := m.now()
	m.mu.Lock()
	defer m.mu.Unlock()
	if m.lastSample.IsZero() {
		return Result{
			DwellSeconds: int64(m.tun.Dwell / time.Second),
			Blockers: []Blocker{{Kind: KindStaleSample,
				Detail: "no reading has been taken yet"}},
		}
	}
	if age := now.Sub(m.lastSample); age > m.tun.MaxSampleGap {
		return Result{
			DwellSeconds: int64(m.tun.Dwell / time.Second),
			Blockers: []Blocker{{Kind: KindStaleSample,
				Detail: fmt.Sprintf("the last reading is %s old (limit %s) — the sampler is not running, so nothing here describes the fleet as it is now",
					roundDur(age), m.tun.MaxSampleGap)}},
		}
	}
	return m.resultLocked(now)
}

func (m *Monitor) resultLocked(now time.Time) Result {
	res := Result{
		Blockers:     append([]Blocker{}, m.blockers...),
		DwellSeconds: int64(m.tun.Dwell / time.Second),
	}
	if len(res.Blockers) > 0 {
		sortBlockers(res.Blockers)
		return res
	}
	held := now.Sub(m.calmSince)
	res.CalmSeconds = int64(held / time.Second)
	if held < m.tun.Dwell {
		res.CalmSeconds = int64(held / time.Second)
		res.Blockers = []Blocker{{
			Kind: KindDwell,
			Detail: fmt.Sprintf("nothing is blocking, but the calm has only held for %s of the %s dwell",
				roundDur(held), m.tun.Dwell),
		}}
		return res
	}
	since := m.calmSince.UnixMilli()
	res.Quiescent = true
	res.Since = &since
	res.Blockers = []Blocker{}
	return res
}

// sortBlockers gives the list a stable order so two identical readings render
// identically — a blocker list that shuffles between polls reads like state
// changing when nothing has.
func sortBlockers(b []Blocker) {
	sort.SliceStable(b, func(i, j int) bool {
		if b[i].Kind != b[j].Kind {
			return b[i].Kind < b[j].Kind
		}
		return b[i].ID < b[j].ID
	})
}
