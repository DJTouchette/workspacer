package limits

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"
	"time"
)

func TestUsageProjectionUsesPaceForAndPreservesIdentity(t *testing.T) {
	now := time.Unix(1788126404, 0)
	cfgs := []PaceConfig{testPace(), {}, testPace(), testPace(), testPace(), testPace()}
	cfgs[2].ExpectedOffsetPct = 2
	cfgs[3].Curve = CurveWorkdays
	cfgs[3].Location, _ = time.LoadLocation("America/Edmonton")
	cfgs[4].ConserveAtRatio = -1
	cfgs[5].WeekendPolicy = WeekendReserve
	cfgs[5].WeekendReservePct = 20
	keys := []*string{ptr(""), ptr("/a/work"), ptr("/b/work"), nil, ptr(`C:\accounts\work`)}
	for _, cfg := range cfgs {
		s := Snapshot{report: WireReport{GeneratedAt: now.Unix(), Providers: []WireProvider{{Provider: "claude"}}}}
		for _, key := range keys {
			s.report.Providers[0].Accounts = append(s.report.Providers[0].Accounts, WireAccount{Account: key, Source: "disk", Windows: WireWindows{FiveHour: paceWin(0, now.Add(150*time.Minute).Unix(), 300), SevenDay: paceWin(70, now.Add(3*24*time.Hour).Unix(), 10080)}})
		}
		out := s.UsageReport(now, cfg, time.Minute)
		if out.ValidUntil != now.Add(time.Minute).Unix() {
			t.Fatal("unbounded sample")
		}
		for i, account := range out.Providers[0].Accounts {
			if !reflect.DeepEqual(account.Account, keys[i]) {
				t.Fatal("account identity collapsed")
			}
			for _, name := range []string{WindowFiveHour, WindowSevenDay} {
				want := PaceFor(bucketFrom("claude", s.report.Providers[0].Accounts[i], name, now), cfg)
				if !reflect.DeepEqual(account.Windows[name].Pace.PaceReport, want) {
					t.Fatal("projection differs from PaceFor")
				}
			}
		}
		raw, err := json.Marshal(out)
		if err != nil {
			t.Fatal(err)
		}
		if !strings.Contains(string(raw), `"usedPct":0`) || !strings.Contains(string(raw), `"value":0`) {
			t.Fatalf("zero lost: %s", raw)
		}
		var wire WireReport
		if err := json.Unmarshal(raw, &wire); err != nil || len(wire.Providers[0].Accounts) != len(keys) {
			t.Fatal("old wire reader failed", err)
		}
	}
}
func ptr(s string) *string { return &s }

func TestUsageProjectionGuards(t *testing.T) {
	now := time.Unix(1788126404, 0)
	for _, tc := range []struct {
		name   string
		change func(*WireAccount)
	}{
		{"rollover", func(a *WireAccount) { a.Windows.FiveHour.ResetsAt = &[]int64{now.Unix()}[0] }},
		{"old daemon", func(a *WireAccount) { a.Windows.FiveHour.WindowMinutes = nil }},
		{"stale", func(a *WireAccount) { a.Fresh = &[]bool{false}[0] }},
		{"missing percent", func(a *WireAccount) { a.Windows.FiveHour.UsedPercent = nil }},
		{"unavailable", func(a *WireAccount) { a.Windows.FiveHour.UsedPercent = &Measured{State: MeasuredUnavailable} }},
	} {
		t.Run(tc.name, func(t *testing.T) {
			a := WireAccount{Windows: WireWindows{FiveHour: paceWin(0, now.Add(150*time.Minute).Unix(), 300)}}
			tc.change(&a)
			s := Snapshot{report: WireReport{Providers: []WireProvider{{Provider: "claude", Accounts: []WireAccount{a}}}}}
			out := s.UsageReport(now, testPace(), time.Minute)
			if out.Providers[0].Accounts[0].Windows[WindowFiveHour].Pace.Known {
				t.Fatal("unknown became known")
			}
		})
	}
}
