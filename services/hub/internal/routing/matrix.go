// Package routing owns the ROUTING MATRIX: the hand-editable file that says
// which capability each work role needs, which real (provider, model, effort) a
// capability resolves to under the active profile, what is known about each
// provider's capacity, the thresholds that pick a routing mode, and the ceiling
// on what a spawn started in a given project directory may be given.
//
// This package LOADS AND VALIDATES the matrix. It does not act on it: there is
// no policy ladder here, no limit reading, no selection, and no enforcement.
// Those are separate pieces that read what this one holds.
//
// WHERE IT LIVES AND WHY. `<user-config-dir>/workspacer-hub/routing.yaml`, 0600,
// beside jobs.json — the hub's own host-trusted state, never the library (which
// a bus caller may write into and the Fleet Manager holds `library.save` for),
// never the layout document (world-readable, client-broadcast), and never
// config.yaml (`config.save` is operator-tier, which the Fleet Manager also
// holds, and its wholesale-path trap is documented at length in
// contracts/README.md). The precedent is internal/jobs, and it is nearly exact:
// a job is persisted argv, and a matrix that decides how much capability and how
// much autonomy a spawned agent gets is host-trusted state by the same rule.
//
// Two things follow from that placement and both are load-bearing:
//
//  1. There is NO routing write RPC over the bus, and there must never be one.
//     That, plus the secret gate refusing the hub's state directory to fs.write,
//     is the whole security argument for the `ceilings:` block.
//  2. This layer is hub-native Go with NO TypeScript twin. Nothing routing-shaped
//     belongs in apps/desktop, cmd/brain or claudemon.
//
// THE FAILURE POLICY IS THE FEATURE. A file that cannot be read, or that does
// not parse, leaves the running matrix EXACTLY as it was and logs; and because
// the user's document is deep-merged OVER the defaults compiled into the binary,
// a merge can never delete. An unresolvable role falls back to the shipped
// default, never to nothing. Disabling something has one spelling and it is an
// explicit `enabled: false`.
package routing

import (
	"fmt"
	"path/filepath"
	"strings"
)

// Assignment is one capability resolved to something spawnable.
type Assignment struct {
	// Provider is a workspacer provider id — claude, codex, copilot, opencode,
	// pi. The spec's vendor names ("openai", "anthropic") are accepted in the
	// file and normalized here at load; see normalizeProvider.
	Provider string `yaml:"provider" json:"provider"`
	Model    string `yaml:"model" json:"model"`
	// Effort is the provider's own reasoning-effort level. The ladders are NOT
	// interchangeable (claude low..max, codex minimal..xhigh), which is why this
	// is validated against the live catalog rather than against a list here.
	Effort string `yaml:"effort,omitempty" json:"effort,omitempty"`
	// Fresh says the worker must not inherit the previous agent's conversation.
	// It is what makes a same-family reviewer an actual reviewer.
	Fresh bool `yaml:"fresh,omitempty" json:"fresh,omitempty"`
	// Enabled is a POINTER so "absent" and "false" are different answers: a deep
	// merge cannot delete, so taking an entry out of service has to be something
	// the user writes, not something they omit. nil means enabled.
	Enabled *bool `yaml:"enabled,omitempty" json:"enabled,omitempty"`
}

// IsEnabled reports whether this entry is in service. Absent means yes.
func (a Assignment) IsEnabled() bool { return a.Enabled == nil || *a.Enabled }

// Profile maps a capability name to the assignment that serves it.
type Profile map[string]Assignment

// Provider is what is known about a provider's CAPACITY — not what it can do.
type Provider struct {
	// Metered says this provider exposes a subscription allowance worth routing
	// around. Only claude and codex report usage windows today.
	Metered bool `yaml:"metered" json:"metered"`
	// WhenUnknown is the health to assume with no trustworthy limit information:
	// "yellow" for a metered provider (do not pretend to know), "unmetered" when
	// the question does not apply.
	WhenUnknown string `yaml:"when_unknown" json:"whenUnknown"`
	Enabled     *bool  `yaml:"enabled,omitempty" json:"enabled,omitempty"`
}

// IsEnabled reports whether this provider is in service. Absent means yes.
func (p Provider) IsEnabled() bool { return p.Enabled == nil || *p.Enabled }

// SpendDown is the rule that turns remaining-but-expiring allowance into
// confidence. All three arms must hold.
type SpendDown struct {
	TimeToResetMinutes        float64 `yaml:"time_to_reset_minutes" json:"timeToResetMinutes"`
	MinRemainingPct           float64 `yaml:"min_remaining_pct" json:"minRemainingPct"`
	MaxForecastPctOfRemaining float64 `yaml:"max_forecast_pct_of_remaining" json:"maxForecastPctOfRemaining"`
}

// Health is where a bucket stops being GREEN.
type Health struct {
	YellowAtUsedPct float64 `yaml:"yellow_at_used_pct" json:"yellowAtUsedPct"`
	RedAtUsedPct    float64 `yaml:"red_at_used_pct" json:"redAtUsedPct"`
}

// Thresholds are the mode rules, kept as configuration rather than as constants
// buried in Go.
type Thresholds struct {
	SpendDown SpendDown `yaml:"spend_down" json:"spendDown"`
	Health    Health    `yaml:"health" json:"health"`
}

// Modes is the manual override: "auto" defers to Thresholds.
type Modes struct {
	Global    string            `yaml:"global" json:"global"`
	Providers map[string]string `yaml:"providers" json:"providers"`
}

// Ceiling is the most a spawn started somewhere may be given.
//
// It can only LOWER what a caller asked for; it never assigns anything. Loading
// it is this package's job — ENFORCING it belongs at the single spawn-path site
// that is not a twin (sanitizeSpawnParams in internal/bus), and nothing here
// reaches it.
type Ceiling struct {
	// MaxCapability is the highest capability a spawn there may resolve to, as a
	// name from Matrix.Capabilities.
	MaxCapability string `yaml:"max_capability" json:"maxCapability"`
	// MaxToolScope is the highest AUTHORITY tier a worker there may be handed:
	// view | triage | operator. "tier" already means authority in this codebase,
	// which is why the model axis is called "capability" everywhere else here.
	MaxToolScope string `yaml:"max_tool_scope" json:"maxToolScope"`
}

// CeilingDefaultKey is the entry that applies to a directory with no entry of
// its own.
const CeilingDefaultKey = "default"

// Issue is one load-time finding: a model the provider does not serve, a role
// pointing at a capability no profile resolves, an unknown provider id. Issues
// never stop a matrix from being applied — they are reported so an operator
// learns at load rather than at spawn time.
type Issue struct {
	// Where is a dotted path into the document, e.g. "profiles.mixed.frontier".
	Where string `json:"where"`
	// Detail says what is wrong, in one sentence.
	Detail string `json:"detail"`
}

func (i Issue) String() string { return i.Where + ": " + i.Detail }

// Matrix is the merged, validated document.
type Matrix struct {
	Version         int                 `yaml:"version" json:"version"`
	ActiveProfile   string              `yaml:"active_profile" json:"activeProfile"`
	Capabilities    []string            `yaml:"capabilities" json:"capabilities"`
	Roles           map[string]string   `yaml:"roles" json:"roles"`
	Profiles        map[string]Profile  `yaml:"profiles" json:"profiles"`
	Providers       map[string]Provider `yaml:"providers" json:"providers"`
	Thresholds      Thresholds          `yaml:"thresholds" json:"thresholds"`
	ForecastWeights map[string]float64  `yaml:"forecast_weights" json:"forecastWeights"`
	Modes           Modes               `yaml:"modes" json:"modes"`
	// ModeShifts is mode -> role -> capability: what a routing mode does to the
	// `roles:` table above. It is keyed by ROLE rather than by capability
	// because the spec's own §12/§13 tables are per-role and disagree per role
	// about the same capability — CONSERVE moves a scout down from `balanced`
	// while leaving the fixer on it, so a capability->capability map would move
	// both and be wrong about one of them.
	ModeShifts map[string]map[string]string `yaml:"mode_shifts" json:"modeShifts"`
	Ceilings   map[string]Ceiling           `yaml:"ceilings" json:"ceilings"`

	// Source is the on-disk file merged in, or "" when only the compiled-in
	// defaults are live (no file, or a file that could not be read or parsed).
	Source string `yaml:"-" json:"source"`
	// Applied lists every dotted key path the user's file carries, sorted.
	Applied []string `yaml:"-" json:"applied"`
	// Changed is the subset of Applied whose value differs from the shipped
	// default — the keys that actually moved the running matrix, and the ones
	// the load log names. A matrix that silently half-applied is worse than one
	// that refused, and this is the list that makes it visible.
	Changed []string `yaml:"-" json:"changed"`
	// Unrecognized lists user keys that match nothing in the shipped defaults.
	// They are still merged (the schema grows, and refusing them would make an
	// older hub reject a newer file), but they are almost always typos.
	Unrecognized []string `yaml:"-" json:"unrecognized"`
	// Issues are the validation findings. See Issue.
	Issues []Issue `yaml:"-" json:"issues"`

	// fallback is the pure compiled-in matrix, used when the user's document
	// points a role or the active profile at something that does not resolve.
	// nil on the defaults matrix itself, which is what stops the recursion.
	fallback *Matrix
}

// Resolved is one role resolved all the way to something spawnable, with the
// reasoning kept so a decision can be explained.
type Resolved struct {
	Role       string     `json:"role"`
	Capability string     `json:"capability"`
	Profile    string     `json:"profile"`
	Assignment Assignment `json:"assignment"`
	// FellBack is set when the user's document could not answer and the shipped
	// default did. Never silently: the caller can log or surface it.
	FellBack bool `json:"fellBack"`
}

// ActiveProfileName is the profile in force, falling back to the shipped
// default's when the file names one that does not exist.
func (m *Matrix) ActiveProfileName() (name string, fellBack bool) {
	if _, ok := m.Profiles[m.ActiveProfile]; ok {
		return m.ActiveProfile, false
	}
	if m.fallback != nil {
		if _, ok := m.Profiles[m.fallback.ActiveProfile]; ok {
			return m.fallback.ActiveProfile, true
		}
	}
	return m.ActiveProfile, false
}

// Capability returns the capability a role requires.
func (m *Matrix) Capability(role string) (string, bool) {
	c, ok := m.Roles[role]
	return c, ok && c != ""
}

// ResolveCapability answers what a capability means under a named profile.
func (m *Matrix) ResolveCapability(profile, capability string) (Assignment, error) {
	p, ok := m.Profiles[profile]
	if !ok {
		return Assignment{}, fmt.Errorf("no profile %q", profile)
	}
	a, ok := p[capability]
	if !ok {
		return Assignment{}, fmt.Errorf("profile %q does not resolve capability %q", profile, capability)
	}
	return a, nil
}

// ResolveRole is the consumer this whole package exists for: role -> capability
// -> the active profile's assignment.
//
// Every arm that cannot be answered from the user's document is answered from
// the compiled-in defaults instead, and says so through Resolved.FellBack. That
// is the "an unresolvable role falls back to the shipped default, never to
// nothing" rule, and it is why a deep merge — which cannot delete — is the right
// merge for this file.
func (m *Matrix) ResolveRole(role string) (Resolved, error) {
	profile, fellBack := m.ActiveProfileName()
	capability, ok := m.Capability(role)
	if !ok {
		if m.fallback == nil {
			return Resolved{}, fmt.Errorf("no role %q", role)
		}
		if capability, ok = m.fallback.Capability(role); !ok {
			return Resolved{}, fmt.Errorf("no role %q", role)
		}
		fellBack = true
	}
	a, err := m.ResolveCapability(profile, capability)
	if err != nil {
		if m.fallback == nil {
			return Resolved{}, err
		}
		// The user pointed the role at a capability their profile does not
		// resolve. Take the SHIPPED role->capability edge rather than inventing
		// one, then resolve that.
		fb, ok := m.fallback.Capability(role)
		if !ok {
			return Resolved{}, err
		}
		a, err = m.ResolveCapability(profile, fb)
		if err != nil {
			return Resolved{}, err
		}
		capability, fellBack = fb, true
	}
	return Resolved{Role: role, Capability: capability, Profile: profile, Assignment: a, FellBack: fellBack}, nil
}

// ModeFor is the routing mode in force for a provider: its own entry when it has
// one, otherwise the global one, otherwise "auto".
func (m *Matrix) ModeFor(provider string) string {
	if v, ok := m.Modes.Providers[normalizeProvider(provider)]; ok && strings.TrimSpace(v) != "" {
		return v
	}
	if strings.TrimSpace(m.Modes.Global) != "" {
		return m.Modes.Global
	}
	return "auto"
}

// ShiftFor answers what a routing mode does to a role's capability, and whether
// it does anything at all.
//
// Absent means unchanged, which is why NORMAL carries no block: the matrix's
// `roles:` table IS the normal answer, and restating it under a mode would be
// two places to keep in agreement.
func (m *Matrix) ShiftFor(mode, role string) (string, bool) {
	byRole, ok := m.ModeShifts[strings.ToLower(strings.TrimSpace(mode))]
	if !ok {
		return "", false
	}
	c, ok := byRole[role]
	if !ok || strings.TrimSpace(c) == "" {
		return "", false
	}
	return c, true
}

// ProviderPolicy returns what is known about a provider's capacity.
func (m *Matrix) ProviderPolicy(provider string) (Provider, bool) {
	p, ok := m.Providers[normalizeProvider(provider)]
	return p, ok
}

// CeilingFor answers the ceiling that governs a project directory, and the key
// it matched.
//
// Matching is EXACT, then nearest ancestor directory, then CeilingDefaultKey.
// It is deliberately LEXICAL over an already-resolved path: this function does
// not canonicalize, does not resolve symlinks, and must never be handed a
// caller-supplied string directly. The enforcement site canonicalizes first —
// that is the same check-path/opened-path rule the filesystem guard is built on,
// and a ceiling looked up on an unresolved path is a ceiling a symlink walks
// around.
func (m *Matrix) CeilingFor(canonicalDir string) (Ceiling, string) {
	if c, ok := m.Ceilings[canonicalDir]; ok {
		return c, canonicalDir
	}
	bestKey := ""
	for key := range m.Ceilings {
		if key == CeilingDefaultKey || !filepath.IsAbs(key) {
			continue
		}
		if !isAtOrInside(key, canonicalDir) {
			continue
		}
		if len(key) > len(bestKey) {
			bestKey = key
		}
	}
	if bestKey != "" {
		return m.Ceilings[bestKey], bestKey
	}
	if c, ok := m.Ceilings[CeilingDefaultKey]; ok {
		return c, CeilingDefaultKey
	}
	if m.fallback != nil {
		if c, ok := m.fallback.Ceilings[CeilingDefaultKey]; ok {
			return c, CeilingDefaultKey
		}
	}
	return Ceiling{}, ""
}

// isAtOrInside is a separator-terminated prefix test between two paths that are
// already resolved. Not filepath.Rel and not a bare strings.HasPrefix: the first
// answers "yes" for a sibling reached by "..", and the second makes
// /home/u/work-old a child of /home/u/work.
func isAtOrInside(dir, target string) bool {
	if target == dir {
		return true
	}
	if !strings.HasSuffix(dir, string(filepath.Separator)) {
		dir += string(filepath.Separator)
	}
	return strings.HasPrefix(target, dir)
}
