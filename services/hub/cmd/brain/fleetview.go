package main

// The fleet as the agent-facing capabilities see it: one decoded view over the
// live session store, shared by agents.reportProgress, agents.notifyWhen,
// agents.close, agents.orphans and agents.reparent.
//
// TWIN: the structural subsets the desktop's services declare against its own
// store — ReportableSession (progressReports.ts) and WatchableSession
// (thresholdWatch.ts). Both are "a row of claudeSessionStore, narrowed"; this
// is the same narrowing over the row the brain actually holds, which is
// claudemon's snapshot plus enrich.go's overlay.

import (
	"context"
	"encoding/json"
	"math"
	"strconv"
	"strings"
	"time"
)

const contextHealthMaxAge = 2 * time.Minute

const maxExactJSONInteger = uint64(9_007_199_254_740_991)

// telemetryEpoch is a decimal string on the current wire. Its source is a
// nanosecond-seeded Rust u64, far beyond float64/JavaScript's exact integer
// range; keeping the digits as text is what makes adjacent increments distinct.
// Legacy numeric epochs are accepted only when they were exactly representable.
type telemetryEpoch string

func (e *telemetryEpoch) UnmarshalJSON(data []byte) error {
	*e = ""
	var text string
	if err := json.Unmarshal(data, &text); err == nil {
		if validTelemetryEpoch(text) {
			*e = telemetryEpoch(text)
		}
		return nil
	}
	var number json.Number
	if err := json.Unmarshal(data, &number); err != nil {
		return nil
	}
	value, err := strconv.ParseUint(number.String(), 10, 64)
	if err == nil && value > 0 && value <= maxExactJSONInteger {
		*e = telemetryEpoch(number.String())
	}
	return nil
}

func validTelemetryEpoch(value string) bool {
	if value == "" || value[0] == '0' {
		return false
	}
	_, err := strconv.ParseUint(value, 10, 64)
	return err == nil
}

type contextHealthReading struct {
	UsedTokens   float64        `json:"usedTokens"`
	WindowTokens float64        `json:"windowTokens"`
	UsedPct      float64        `json:"usedPct"`
	WindowSource string         `json:"windowSource"`
	ObservedAt   string         `json:"observedAt"`
	Epoch        telemetryEpoch `json:"epoch"`
	Provider     string         `json:"provider"`
}

type rawContextHealthReading struct {
	UsedTokens   float64        `json:"used_tokens"`
	WindowTokens float64        `json:"window_tokens"`
	UsedPct      float64        `json:"used_pct"`
	WindowSource string         `json:"window_source"`
	ObservedAt   string         `json:"observed_at"`
	Epoch        telemetryEpoch `json:"epoch"`
	Provider     string         `json:"provider"`
}

// fleetSession is one session as the fleet verbs read it.
//
// The json tags name the ENRICHED/compat field names (enrich.go), not
// claudemon's raw ones, because that is what the store holds: `sessionId`,
// `status`, `ambientState`, `lastActivity`, `parentSessionId`, `label` and
// `isWakeTarget` are all written by enrichAndCompat before a row lands.
type fleetSession struct {
	SessionID       string `json:"sessionId"`
	Cwd             string `json:"cwd"`
	Label           string `json:"label"`
	Status          string `json:"status"`
	AmbientState    string `json:"ambientState"`
	ParentSessionID string `json:"parentSessionId"`
	IsWakeTarget    bool   `json:"isWakeTarget"`
	LastActivity    int64  `json:"lastActivity"`
	Provider        string `json:"provider"`

	// Usage is the compat overlay's camelCase block. It carries costUSD but NOT
	// the cumulative token counters — those live only on the status line, which
	// is why the fallback below is not optional decoration.
	Usage *struct {
		CostUSD *float64 `json:"costUSD"`
	} `json:"usage"`

	// StatusLine is the overlay's camelCase status line, and RawStatusLine is
	// claudemon's snake_case original.
	//
	// BOTH are read, and the raw one WINS, because sessionStore.updateStatusLine
	// merges a fresh status line into `status_line` ONLY — it does not re-run
	// the compat overlay. So on every high-frequency statusline tick the camel
	// block goes stale while the snake one is current, and a threshold watch
	// reading only the camel block would compare against whatever the numbers
	// were at the last full snapshot. That is the difference between catching a
	// runaway spend and noticing it by chance.
	StatusLine *struct {
		TotalInputTokens  *float64              `json:"totalInputTokens"`
		TotalOutputTokens *float64              `json:"totalOutputTokens"`
		CostUSD           *float64              `json:"costUSD"`
		ContextHealth     *contextHealthReading `json:"contextHealth"`
		// OverageOutOfCredits is the daemon's structured out-of-credits bit,
		// read by the finish wake's failure check (workerfailure.go). Standing
		// ACCOUNT state, not a per-turn event — it only ever enriches a failure
		// the error marker already established.
		OverageOutOfCredits *bool `json:"overageOutOfCredits"`
	} `json:"statusLine"`
	RawStatusLine *struct {
		TotalInputTokens    *float64                 `json:"total_input_tokens"`
		TotalOutputTokens   *float64                 `json:"total_output_tokens"`
		CostUSD             *float64                 `json:"cost_usd"`
		ContextHealth       *rawContextHealthReading `json:"context_health"`
		OverageOutOfCredits *bool                    `json:"overage_out_of_credits"`
	} `json:"status_line"`
}

// contextHealth returns only fresh, internally consistent runtime evidence.
// Raw wins because high-frequency updates refresh only status_line; a present
// but malformed raw sample must fail closed rather than fall back to stale
// camelCase compatibility data.
func (s fleetSession) contextHealth(now time.Time) *contextHealthReading {
	var h *contextHealthReading
	if s.RawStatusLine != nil && s.RawStatusLine.ContextHealth != nil {
		raw := s.RawStatusLine.ContextHealth
		h = &contextHealthReading{
			UsedTokens: raw.UsedTokens, WindowTokens: raw.WindowTokens, UsedPct: raw.UsedPct,
			WindowSource: raw.WindowSource, ObservedAt: raw.ObservedAt, Epoch: raw.Epoch,
			Provider: raw.Provider,
		}
	} else if s.StatusLine != nil && s.StatusLine.ContextHealth != nil {
		copy := *s.StatusLine.ContextHealth
		h = &copy
	}
	if h == nil || h.WindowSource != "runtime" || h.Provider == "" ||
		(s.Provider != "" && h.Provider != s.Provider) || h.WindowTokens <= 0 ||
		h.UsedTokens < 0 || h.UsedTokens > h.WindowTokens || h.Epoch == "" ||
		math.IsNaN(h.UsedPct) || math.IsInf(h.UsedPct, 0) || h.UsedPct < 0 || h.UsedPct > 100 {
		return nil
	}
	observed, err := time.Parse(time.RFC3339, h.ObservedAt)
	if err != nil || observed.After(now.Add(5*time.Second)) || now.Sub(observed) > contextHealthMaxAge {
		return nil
	}
	computed := h.UsedTokens / h.WindowTokens * 100
	if math.Abs(computed-h.UsedPct) > 0.01 {
		return nil
	}
	return h
}

// outOfCredits reads the daemon's out-of-credits bit, preferring the RAW block
// over the compat overlay for the same reason `pick` does: updateStatusLine
// merges a fresh status line into `status_line` only, so the camelCase copy goes
// stale between full snapshots.
func (s fleetSession) outOfCredits() bool {
	if s.RawStatusLine != nil && s.RawStatusLine.OverageOutOfCredits != nil {
		return *s.RawStatusLine.OverageOutOfCredits
	}
	if s.StatusLine != nil && s.StatusLine.OverageOutOfCredits != nil {
		return *s.StatusLine.OverageOutOfCredits
	}
	return false
}

// ended reports whether this row is finished. TWIN: the `status === 'ended'`
// test both desktop services make; compatSnapshot maps claudemon's "stopped"
// mode onto it.
func (s fleetSession) ended() bool { return s.Status == "ended" }

// displayLabel is the wake bullet's label: the session's own name, else the
// basename of its cwd, else "Agent".
func (s fleetSession) displayLabel() string {
	if s.Label != "" {
		return s.Label
	}
	if base := fleetAgentLabel(s.Cwd); base != "" {
		return base
	}
	return "Agent"
}

// tokens is the session's CUMULATIVE token spend — the number a manager means
// by "how big has this got". Not the point-in-time context window: a worker
// that compacted twice has spent the tokens either way.
// TWIN: sessionTokens in thresholdWatch.ts.
func (s fleetSession) tokens() float64 {
	in := s.pick(func(t *float64, _ *float64, _ *float64) *float64 { return t })
	out := s.pick(func(_ *float64, t *float64, _ *float64) *float64 { return t })
	return in + out
}

// costUSD is the session's cumulative cost. `usage.costUSD` is the compat
// overlay's own field and is checked first for parity with the desktop, which
// reads `usage?.costUSD ?? statusLine?.costUSD`; the status-line fallback is
// what covers the managed providers (codex/copilot/opencode/pi), which never populate
// usage at all.
func (s fleetSession) costUSD() float64 {
	if s.Usage != nil && s.Usage.CostUSD != nil {
		return *s.Usage.CostUSD
	}
	return s.pick(func(_ *float64, _ *float64, c *float64) *float64 { return c })
}

// pick reads one status-line number, preferring the RAW (always-current) block
// over the compat overlay. See RawStatusLine's comment for why that order.
func (s fleetSession) pick(sel func(in, out, cost *float64) *float64) float64 {
	if s.RawStatusLine != nil {
		if v := sel(s.RawStatusLine.TotalInputTokens, s.RawStatusLine.TotalOutputTokens, s.RawStatusLine.CostUSD); v != nil {
			return *v
		}
	}
	if s.StatusLine != nil {
		if v := sel(s.StatusLine.TotalInputTokens, s.StatusLine.TotalOutputTokens, s.StatusLine.CostUSD); v != nil {
			return *v
		}
	}
	return 0
}

// fleetSessions decodes the live store into the fleet view.
//
// It reads the FULL store, not visibleSnapshots(): the visibility rule
// (visibility.go) is about what a client's SIDEBAR shows, and hiding a stopped
// session from a list is not the same claim as "this session does not exist".
// Routing a worker's report to its parent, or refusing to reparent onto a dead
// manager, has to be decided on what is actually there — otherwise a manager
// the user happened to hide becomes an unreachable recipient and the refusal
// says the wrong thing.
//
// A brain with no live store (catalog scope) has no fleet: these capabilities
// are full-scope only, so the empty slice is the honest answer and every caller
// below turns it into a named refusal rather than a silent no-op.
func (r *registry) fleetSessions(_ context.Context) []fleetSession {
	if r.store == nil {
		return nil
	}
	snaps := r.store.all()
	out := make([]fleetSession, 0, len(snaps))
	for _, snap := range snaps {
		var s fleetSession
		if json.Unmarshal(snap, &s) != nil {
			continue
		}
		if s.SessionID == "" {
			// Pre-overlay rows (a raw claudemon snapshot that never went through
			// enrichAndCompat) carry only session_id. Fall back rather than drop
			// the row: an invisible session is a wrong answer, not a missing one.
			s.SessionID = snapshotID(snap)
		}
		if s.SessionID == "" {
			continue
		}
		out = append(out, s)
	}
	return out
}

// findFleetSession returns the row for one id.
func findFleetSession(all []fleetSession, id string) (fleetSession, bool) {
	for _, s := range all {
		if s.SessionID == id {
			return s, true
		}
	}
	return fleetSession{}, false
}

// deliverFleetWake sends a composed wake to a session through claudemon's
// queued /message endpoint — the same sink every other fleet wake uses, and
// deliberately NOT the PTY write claude.answer makes.
//
// It reports the daemon's "this session has ended" (409) as an error rather
// than swallowing it: a worker that believes it reported and did not is exactly
// the failure report_progress exists to prevent.
func (r *registry) deliverFleetWake(ctx context.Context, sessionID, text string) error {
	ok, err := r.cm.submitMessage(ctx, sessionID, text)
	if err != nil {
		return err
	}
	if !ok {
		return errFleetRecipientEnded
	}
	return nil
}

// errFleetRecipientEnded is the daemon refusing input for an ended session.
var errFleetRecipientEnded = &fleetError{"the recipient session has ended and cannot accept messages"}

type fleetError struct{ msg string }

func (e *fleetError) Error() string { return e.msg }

// flattenLine collapses all whitespace to single spaces and trims.
//
// The bullet grammar is LINE-based, so a note spanning lines does not merely
// look bad — it makes the wake unparseable, and parseFleetMessage returns null
// for a message whose bullets do not all parse, demoting the whole card to a
// text blob. TWIN: flattenNote in progressReports.ts (`/\s+/g` → ' ').
func flattenLine(s string) string {
	return strings.Join(strings.Fields(s), " ")
}
