package limits

import "time"

// UsageProjection is the narrow, read-only Overview wire. No spend, tokens,
// configuration, or decisions cross this seam. ValidUntil bounds sampled pace,
// independently of provider observation freshness and each window's reset.
type UsageProjection struct {
	GeneratedAt int64           `json:"generated_at"`
	EvaluatedAt int64           `json:"evaluated_at"`
	ValidUntil  int64           `json:"valid_until"`
	Providers   []UsageProvider `json:"providers"`
}
type UsageProvider struct {
	Provider string         `json:"provider"`
	Note     *string        `json:"note"`
	Accounts []UsageAccount `json:"accounts"`
}
type UsageAccount struct {
	WireAccount
	Windows map[string]*UsageWindow `json:"windows"`
}
type UsageWindow struct {
	WireWindow
	Pace UsagePace `json:"pace"`
}

// Explicit numeric fields preserve known zero despite PaceReport's omitempty.
type UsagePace struct {
	PaceReport
	UsedPct     float64 `json:"usedPct"`
	ExpectedPct float64 `json:"expectedPct"`
}

func (s Snapshot) UsageReport(now time.Time, cfg PaceConfig, validity time.Duration) UsageProjection {
	out := UsageProjection{GeneratedAt: s.report.GeneratedAt, EvaluatedAt: now.Unix(), ValidUntil: now.Add(validity).Unix(), Providers: []UsageProvider{}}
	buckets := s.Buckets(now)
	i := 0
	for _, p := range s.report.Providers {
		provider := UsageProvider{Provider: p.Provider, Note: p.Note, Accounts: []UsageAccount{}}
		for _, a := range p.Accounts {
			account := UsageAccount{WireAccount: a, Windows: map[string]*UsageWindow{}}
			for _, name := range WindowOrder {
				b := buckets[i]
				i++
				if w := a.Windows.Window(name); w != nil {
					pace := PaceFor(b, cfg)
					account.Windows[name] = &UsageWindow{WireWindow: *w, Pace: UsagePace{PaceReport: pace, UsedPct: pace.UsedPct, ExpectedPct: pace.ExpectedPct}}
				}
			}
			provider.Accounts = append(provider.Accounts, account)
		}
		out.Providers = append(out.Providers, provider)
	}
	return out
}
