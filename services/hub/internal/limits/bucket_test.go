package limits

import (
	"os"
	"testing"
	"time"
)

// testdata/usage-report.json is a REAL GET /usage/report document, captured
// from the live daemon on 2026-08-30 with home directories rewritten to
// /home/you and nothing else changed. It is here rather than in contracts/
// because it pins no cross-language behaviour — it is the wire shape this
// decoder must survive, and a hand-written sample would only ever contain the
// cases somebody thought of. This one contains the three that matter and were
// not invented: a claude account whose OAuth token has expired (every window
// `unknown` WITH a reason), a claude row the daemon cannot attribute to any
// account at all (`account: null`, which is NOT the default login), and a
// copilot row whose windows are permanently `unavailable`.
const liveReportPath = "testdata/usage-report.json"

// The document's own generated_at. Every assertion below is made against a
// fixed clock derived from it, so the sweep does not change meaning with the
// wall clock.
const liveGeneratedAt = 1788126404

func loadLiveReport(t *testing.T) Snapshot {
	t.Helper()
	raw, err := os.ReadFile(liveReportPath)
	if err != nil {
		t.Fatalf("read %s: %v", liveReportPath, err)
	}
	snap, err := DecodeReport(raw, time.Unix(liveGeneratedAt, 0))
	if err != nil {
		t.Fatalf("decode %s: %v", liveReportPath, err)
	}
	if snap.Empty() {
		t.Fatal("the captured report decoded to zero providers — a field was renamed and this whole file is asserting nothing")
	}
	return snap
}

func find(t *testing.T, buckets []Bucket, id string) Bucket {
	t.Helper()
	for _, b := range buckets {
		if b.ID() == id {
			return b
		}
	}
	t.Fatalf("no bucket %q in %d decoded buckets", id, len(buckets))
	return Bucket{}
}

func TestDecodeRealUsageReport(t *testing.T) {
	snap := loadLiveReport(t)
	now := time.Unix(liveGeneratedAt, 0)
	buckets := snap.Buckets(now)

	// Three providers, three windows each, one bucket per ACCOUNT — claude has
	// three account rows on this machine and folding them together is the
	// failure this key exists to prevent.
	if got, want := len(buckets), 5*len(WindowOrder); got != want {
		t.Fatalf("decoded %d buckets, want %d (5 account rows x %d windows)", got, want, len(WindowOrder))
	}
	if got := snap.Providers(); len(got) != 3 {
		t.Errorf("providers = %v, want three", got)
	}
	if snap.GeneratedAt.Unix() != liveGeneratedAt {
		t.Errorf("generated_at = %v", snap.GeneratedAt)
	}

	t.Run("a healthy running window is usable", func(t *testing.T) {
		b := find(t, buckets, "claude//five_hour")
		if !b.AccountKnown || b.Account != "" {
			t.Errorf("account = %q known=%v, want the default login spelled \"\"", b.Account, b.AccountKnown)
		}
		if b.AccountLabel != "default" || !b.IsDefault {
			t.Errorf("label=%q isDefault=%v", b.AccountLabel, b.IsDefault)
		}
		if b.Source != "oauth_poll" {
			t.Errorf("source = %q", b.Source)
		}
		if b.ObservedAt == nil || b.Fresh == nil || !*b.Fresh {
			t.Errorf("provenance lost: observedAt=%v fresh=%v", b.ObservedAt, b.Fresh)
		}
		used, ok := b.Reading.UsedPercent()
		if !ok || used != 18 {
			t.Fatalf("UsedPercent() = %v, %v; want 18, true", used, ok)
		}
		if r, _ := b.Reading.RemainingPercent(); r != 82 {
			t.Errorf("RemainingPercent() = %v, want 82", r)
		}
		ttr, ok := b.Reading.TimeToReset()
		if !ok || ttr != 5596*time.Second {
			t.Errorf("TimeToReset() = %v, %v; want 5596s", ttr, ok)
		}
		// Anthropic reports no window_minutes. A healthy reading with no
		// declared length must not read as unreadable.
		if d, ok := b.Reading.WindowLength(); ok {
			t.Errorf("WindowLength() = %v, and claude declares none", d)
		}
		if !b.Metered() {
			t.Error("a provider serving a live percentage is metered")
		}
	})

	t.Run("an expired token is unknown WITH a reason, not zero", func(t *testing.T) {
		b := find(t, buckets, "claude//home/you/.claude/accounts/work/five_hour")
		if b.Reading.Usable() {
			t.Error("a window with no reset time is never usable")
		}
		if b.Reading.Reason() != ReasonNoResetTime {
			t.Errorf("reason = %q", b.Reading.Reason())
		}
		if _, ok := b.Reading.UsedPercent(); ok {
			t.Error("no percentage may be read off it")
		}
		if b.DisplayOnlyRawUsedPercent().State != MeasuredUnknown || b.DisplayOnlyRawUsedPercent().Reason == "" {
			t.Errorf("raw scalar = %+v; unknown must arrive WITH its reason, which is what makes it retryable rather than permanent", b.DisplayOnlyRawUsedPercent())
		}
		if b.Failure == nil || b.Failure.Kind != "needs_reauth" {
			t.Errorf("failure = %+v, want the classified needs_reauth record", b.Failure)
		}
		// Retryable, so still metered: this account HAS an allowance, the
		// daemon just cannot read it right now.
		if !b.Metered() {
			t.Error("a re-authable account is metered — its allowance exists")
		}
	})

	t.Run("an unattributable row is not the default account", func(t *testing.T) {
		b := find(t, buckets, "claude/?/five_hour")
		if b.AccountKnown {
			t.Fatal("account null must decode as UNKNOWN, never as the default login")
		}
		if b.AccountLabel != "unattributed" {
			t.Errorf("label = %q", b.AccountLabel)
		}
		if b.Source != "transcript" {
			t.Errorf("source = %q", b.Source)
		}
		// And the id must not collide with the default login's.
		if b.ID() == "claude//five_hour" {
			t.Fatal("the unknown account and the default login share a bucket id")
		}
	})

	t.Run("copilot is structurally unmetered, permanently", func(t *testing.T) {
		for _, w := range WindowOrder {
			b := find(t, buckets, "copilot//home/you/.copilot/session-store.db/"+w)
			if b.Reading.Usable() {
				t.Errorf("%s: usable, and GitHub publishes no quota at all", w)
			}
			if b.DisplayOnlyRawUsedPercent().State != MeasuredUnavailable {
				t.Errorf("%s: raw state = %q, want unavailable — a retry will never succeed and collapsing that into `unknown` is how a provider gets conserved forever", w, b.DisplayOnlyRawUsedPercent().State)
			}
			if b.Metered() {
				t.Errorf("%s: metered, and there is no allowance here to conserve", w)
			}
		}
	})

	t.Run("a window the source did not include is unknown, not zero", func(t *testing.T) {
		b := find(t, buckets, "codex//home/you/.codex/five_hour")
		if b.Reading.Usable() {
			t.Error("no reset time, so not usable")
		}
		if _, ok := b.Reading.UsedPercent(); ok {
			t.Error("no percentage")
		}
		if b.DisplayOnlyRawUsedPercent().State != MeasuredUnknown {
			t.Errorf("raw state = %q, want unknown (the rollout carrying rate limits did not include this window — retryable)", b.DisplayOnlyRawUsedPercent().State)
		}
	})

	t.Run("a genuine zero is an answer", func(t *testing.T) {
		b := find(t, buckets, "codex//home/you/.codex/seven_day")
		used, ok := b.Reading.UsedPercent()
		if !ok || used != 0 {
			t.Fatalf("UsedPercent() = %v, %v; want 0, true — Measured.ok carries a real zero and a reader that treats it as missing turns a healthy provider into an unknown one", used, ok)
		}
		if d, ok := b.Reading.WindowLength(); !ok || d != 7*24*time.Hour {
			t.Errorf("WindowLength() = %v, %v; want 168h", d, ok)
		}
	})

	t.Run("a provider's note survives", func(t *testing.T) {
		if note, ok := snap.Note("copilot"); !ok || note == "" {
			t.Error("an empty account list plus a reason is a reading; an empty list alone is indistinguishable from a provider nobody uses")
		}
	})

	t.Run("Bucket() finds one row by key", func(t *testing.T) {
		b, ok := snap.Bucket(now, "claude", "", WindowSevenDay)
		if !ok {
			t.Fatal("the default claude login is in the document")
		}
		if used, ok := b.Reading.UsedPercent(); !ok || used != 91 {
			t.Errorf("UsedPercent() = %v, %v; want 91", used, ok)
		}
		if _, ok := snap.Bucket(now, "opencode", "", WindowFiveHour); ok {
			t.Error("opencode is ABSENT from the document — absent is not `unknown at 0%`, and a caller must be able to tell them apart")
		}
	})
}

// TestTheSameSnapshotIsRejudgedAgainstTheCallersClock is the whole reason
// Snapshot holds the wire rather than pre-judged buckets.
//
// A document fetched at T and judged at T is correct. The identical document
// consulted at T+2h contains a window that lapsed in between, and a cached
// verdict would still say 18% used with 93 minutes to go. That is the
// cached-document half of the currency defect, and it is the half a client
// creates for itself: the daemon's own is_current was right when it was
// written.
func TestTheSameSnapshotIsRejudgedAgainstTheCallersClock(t *testing.T) {
	snap := loadLiveReport(t)
	at := func(offset time.Duration) Bucket {
		return find(t, snap.Buckets(time.Unix(liveGeneratedAt, 0).Add(offset)), "claude//five_hour")
	}

	fresh := at(0)
	if !fresh.Reading.Usable() {
		t.Fatal("at the fetch instant the window is running")
	}

	// One second before the reset, and one second after.
	const toReset = 5596 * time.Second
	if b := at(toReset - time.Second); !b.Reading.Usable() {
		t.Error("one second before the reset the window is still running")
	}
	if b := at(toReset); !b.Reading.Usable() {
		// resets_at == now is rolled over, strictly.
		if b.Reading.Reason() != ReasonResetEqualsNow {
			t.Errorf("at exactly the reset: reason = %q, want %q", b.Reading.Reason(), ReasonResetEqualsNow)
		}
	} else {
		t.Error("at exactly the reset the window has closed — the comparison is strictly greater-than, matching codex_usage.rs")
	}
	later := at(2 * time.Hour)
	if later.Reading.Usable() {
		t.Fatal("two hours past the reset the reading is stale and must yield UNKNOWN")
	}
	if later.Reading.Reason() != ReasonResetHasPassed {
		t.Errorf("reason = %q, want %q", later.Reading.Reason(), ReasonResetHasPassed)
	}
	if _, ok := later.Reading.UsedPercent(); ok {
		t.Error("the stale percentage escaped through a cached snapshot — this is the defect, one indirection later")
	}
	// The raw scalar is still reachable so the decision can EXPLAIN itself.
	raw := later.DisplayOnlyRawUsedPercent()
	if v, ok := raw.Number(); !ok || v != 18 {
		t.Errorf("DisplayOnlyRawUsedPercent() = %v, %v; a decision that drops a provider must be able to say what it last saw", v, ok)
	}
	if later.Reading.Explain() == "" {
		t.Error("no explanation for a dropped provider")
	}
}

func TestDecodeReportRefusesGarbage(t *testing.T) {
	if _, err := DecodeReport([]byte("not json"), time.Now()); err == nil {
		t.Fatal("a body that is not the report must be an error, not an empty fleet: no answer and zero usage are different claims")
	}
	// An EMPTY but well-formed document is not an error — it is a daemon with
	// nothing to say — but it must be distinguishable from one that was read.
	snap, err := DecodeReport([]byte(`{"generated_at":0,"providers":[]}`), time.Unix(5, 0))
	if err != nil {
		t.Fatalf("well-formed empty document: %v", err)
	}
	if !snap.Empty() || len(snap.Buckets(time.Unix(5, 0))) != 0 {
		t.Error("an empty document yields no buckets")
	}
	if !snap.GeneratedAt.IsZero() {
		t.Error("generated_at 0 is no timestamp, not the epoch")
	}
}
