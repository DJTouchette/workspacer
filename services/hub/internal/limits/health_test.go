package limits

import (
	"testing"
	"time"
)

// The bands the shipped matrix carries. Written here as a fixture rather than
// as a constant the ladder reads, because the ladder must not have one: 70 and
// 90 are the user's numbers and they live in routing.yaml.
var testBands = Bands{YellowAtUsedPct: 70, RedAtUsedPct: 90}

func at(sec int64) time.Time { return time.Unix(sec, 0) }

// win builds one wire window. resetsAt of 0 means "no reset time reported".
func win(state MeasuredState, value float64, resetsAt int64) *WireWindow {
	w := &WireWindow{UsedPercent: &Measured{State: state}}
	if state == MeasuredOk {
		v := value
		w.UsedPercent.Value = &v
	} else {
		w.UsedPercent.Reason = "a reason, because a state without one is half an answer"
	}
	if resetsAt != 0 {
		r := resetsAt
		w.ResetsAt = &r
	}
	return w
}

func bucketAt(w *WireWindow, window string, now time.Time) Bucket {
	acct := ""
	return bucketFrom("codex", WireAccount{
		Account: &acct, Label: "default", IsDefault: true, Source: "disk",
		Windows: WireWindows{FiveHour: w, SevenDay: w, Monthly: w},
	}, window, now)
}

func TestBucketHealthLadder(t *testing.T) {
	now := at(1788126404)
	future := now.Add(time.Hour).Unix()
	past := now.Add(-time.Hour).Unix()

	for _, tc := range []struct {
		name    string
		window  *WireWindow
		bands   Bands
		want    Health
		metered bool
		why     string
	}{
		{"a clean window is green", win(MeasuredOk, 12, future), testBands, HealthGreen, true, ""},
		{"one point under yellow is still green", win(MeasuredOk, 69.9, future), testBands, HealthGreen, true, ""},
		{"exactly at the yellow threshold is yellow", win(MeasuredOk, 70, future), testBands, HealthYellow, true,
			"the threshold is where the band STARTS; a > comparison makes 70 mean 'not yet constrained' and the matrix's number then describes nothing"},
		{"exactly at the red threshold is red", win(MeasuredOk, 90, future), testBands, HealthRed, true, ""},
		{"a fully consumed allowance is exhausted, not red", win(MeasuredOk, 100, future), testBands, HealthExhausted, true,
			"RED means scarce and EXHAUSTED means route around it; a fleet that only ever sees RED keeps sending work at a provider that will refuse it"},
		{"a stale window is UNKNOWN, never green", win(MeasuredOk, 20, past), testBands, HealthUnknown, true,
			"20% used reads as 80% remaining and would pass every spend-down arm, off a window that closed an hour ago"},
		{"a stale window at 95% is UNKNOWN, never red", win(MeasuredOk, 95, past), testBands, HealthUnknown, true,
			"the phantom-CONSERVE direction: the window has in fact rolled over to ~0% used, and conserving on it is self-reinforcing"},
		{"a running window whose utilization is unreadable is UNKNOWN", win(MeasuredUnknown, 0, future), testBands, HealthUnknown, true,
			"currency and readability are separate axes; answering GREEN here invents the number the source declined to give"},
		{"a permanently unavailable window is UNMETERED", win(MeasuredUnavailable, 0, 0), testBands, HealthUnmetered, false,
			"copilot's 403. Not a dark reading — an absent meter, and conserving it conserves nothing"},
		{"no bands at all is UNKNOWN, not green", win(MeasuredOk, 12, future), Bands{}, HealthUnknown, true,
			"a 0/0 ladder cannot order anything, and every provider reading GREEN against it is the worst possible default"},
		{"inverted bands are UNKNOWN", win(MeasuredOk, 12, future), Bands{YellowAtUsedPct: 90, RedAtUsedPct: 70}, HealthUnknown, true, ""},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got := BucketHealth(bucketAt(tc.window, WindowFiveHour, now), tc.bands)
			if got.Health != tc.want {
				t.Errorf("health = %q, want %q\n  why: %s", got.Health, tc.want, tc.why)
			}
			if got.Metered != tc.metered {
				t.Errorf("metered = %v, want %v", got.Metered, tc.metered)
			}
			if got.Explain == "" {
				t.Error("no explanation — a provider that vanishes from a decision must be able to say why")
			}
			// The invariant that outranks the ladder: no number ever escapes a
			// reading that is not current.
			if got.Health == HealthUnknown && got.State != WindowCurrent {
				if got.UsedPercent != nil || got.RemainingPercent != nil || got.ResetsInSeconds != nil {
					t.Errorf("a non-current bucket carried used=%v remaining=%v resets=%v", got.UsedPercent, got.RemainingPercent, got.ResetsInSeconds)
				}
			}
			if got.ResetsInSeconds != nil && *got.ResetsInSeconds <= 0 {
				t.Errorf("ResetsInSeconds = %d — the spend-down arm reads any non-positive value as 'under 90 minutes'", *got.ResetsInSeconds)
			}
		})
	}
}

// TestWorstNeverFoldsAnUnknownIntoAHealthyProvider is rule 2, and it is the one
// that decides whether the whole layer can be trusted.
func TestWorstNeverFoldsAnUnknownIntoAHealthyProvider(t *testing.T) {
	green := BucketReport{Window: "seven_day", Health: HealthGreen, Metered: true}
	unknown := BucketReport{Window: "five_hour", Health: HealthUnknown, Metered: true, Explain: "rolled over"}
	yellow := BucketReport{Window: "seven_day", Health: HealthYellow, Metered: true, Explain: "75%"}
	red := BucketReport{Window: "seven_day", Health: HealthRed, Metered: true, Explain: "92%"}
	exhausted := BucketReport{Window: "five_hour", Health: HealthExhausted, Metered: true, Explain: "100%"}
	unmetered := BucketReport{Window: "monthly", Health: HealthUnmetered, Metered: false, Explain: "never published"}

	for _, tc := range []struct {
		name string
		in   []BucketReport
		want Health
		why  string
	}{
		{"all green", []BucketReport{green, green}, HealthGreen, ""},
		{"an unreadable short window beats a healthy long one", []BucketReport{unknown, green}, HealthUnknown,
			"THE RULE. A readable weekly window cannot vouch for a five-hour one nobody could read"},
		{"a constrained long window beats an unreadable short one", []BucketReport{unknown, yellow}, HealthYellow,
			"a definite constraint is worse news than an unreadable one, and acting on it is not a guess"},
		{"red beats everything below it", []BucketReport{unknown, green, red}, HealthRed, ""},
		{"exhausted beats red", []BucketReport{red, exhausted}, HealthExhausted, ""},
		{"unmetered windows are skipped, not folded", []BucketReport{green, unmetered}, HealthGreen,
			"codex declares no monthly window; folding that in as UNKNOWN makes every provider permanently unknown"},
		{"a provider whose every window is unmetered is UNMETERED", []BucketReport{unmetered, unmetered}, HealthUnmetered,
			"copilot. There is no allowance here to conserve, which is a different answer from 'we cannot read it'"},
		{"no buckets at all is UNKNOWN", nil, HealthUnknown,
			"opencode and pi never appear in the document — absent is not 0% used"},
	} {
		t.Run(tc.name, func(t *testing.T) {
			got, because := Worst(tc.in)
			if got != tc.want {
				t.Errorf("Worst = %q, want %q\n  why: %s", got, tc.want, tc.why)
			}
			if because == "" {
				t.Error("no sentence behind the fold")
			}
		})
	}
}

// TestForProviderSkipsRowsTheSpawnWillNotBill pins the account dimension: a
// fold that mixed three claude logins would be a lie in either direction, and a
// row the daemon could not attribute is not the default login.
func TestForProviderSkipsRowsTheSpawnWillNotBill(t *testing.T) {
	now := at(1788126404)
	future := now.Add(time.Hour).Unix()
	dflt, work := "", "work"
	// All three window keys, as the real document always carries them: an
	// ABSENT window object is a different (and deliberately UNKNOWN) answer
	// from one that reports `unavailable`, so a fixture that omitted seven_day
	// and monthly would be testing the wrong thing.
	acct := func(key *string, isDefault bool, used float64) WireAccount {
		return WireAccount{
			Account: key, IsDefault: isDefault, Source: "oauth_poll",
			Windows: WireWindows{
				FiveHour: win(MeasuredOk, used, future),
				SevenDay: win(MeasuredOk, 30, now.Add(96*time.Hour).Unix()),
				Monthly:  win(MeasuredUnavailable, 0, 0),
			},
		}
	}
	report := WireReport{GeneratedAt: now.Unix(), Providers: []WireProvider{{
		Provider: "claude",
		Accounts: []WireAccount{
			acct(&dflt, true, 12),
			acct(&work, false, 95),
			acct(nil, false, 40), // the unattributable fold
		},
	}}}
	snap := Snapshot{GeneratedAt: now, FetchedAt: now, report: report}
	buckets := snap.Buckets(now)

	if got := ForProvider(buckets, "claude", "", testBands); got.Health != HealthGreen {
		t.Errorf("the default login = %q, want green (12%% used); a fold across accounts would have picked up the 95%% row", got.Health)
	}
	if got := ForProvider(buckets, "claude", "work", testBands); got.Health != HealthExhausted && got.Health != HealthRed {
		t.Errorf("the work login = %q, want red (95%% used)", got.Health)
	}

	// ResolveAccount: an unnamed request lands on the default row, a named one
	// must exist, and the unattributable row is reachable from neither.
	if a, ok := ResolveAccount(buckets, "claude", ""); !ok || a != "" {
		t.Errorf("ResolveAccount(unnamed) = %q, %v; want the default login", a, ok)
	}
	if a, ok := ResolveAccount(buckets, "claude", "work"); !ok || a != "work" {
		t.Errorf("ResolveAccount(work) = %q, %v", a, ok)
	}
	if _, ok := ResolveAccount(buckets, "claude", "nobody"); ok {
		t.Error("ResolveAccount resolved an account that is not in the document — a decision billed to an allowance nobody has")
	}
	if _, ok := ResolveAccount(buckets, "opencode", ""); ok {
		t.Error("ResolveAccount answered for a provider that is not in the document at all")
	}
}
