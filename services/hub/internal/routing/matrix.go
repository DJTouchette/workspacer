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
	// MinEffort is the FLOOR the mode's effort step may not push this row
	// below — a review capability that is only a review at `high` says so here
	// rather than hoping nothing ever trims it.
	//
	// It is a name from the PROVIDER's own ladder (see effort.go), validated at
	// load against that ladder, and it is honoured on an ALTERNATIVE as well as
	// on a primary: a fallover takes the alternative's own effort, so a floor
	// written only on the primary would stop binding at the exact moment the
	// answer moved. Empty means the ladder's own floor is the only floor.
	MinEffort string `yaml:"min_effort,omitempty" json:"minEffort,omitempty"`
	// Fresh says the worker must not inherit the previous agent's conversation.
	// It is what makes a same-family reviewer an actual reviewer.
	Fresh bool `yaml:"fresh,omitempty" json:"fresh,omitempty"`
	// Enabled is a POINTER so "absent" and "false" are different answers: a deep
	// merge cannot delete, so taking an entry out of service has to be something
	// the user writes, not something they omit. nil means enabled.
	Enabled *bool `yaml:"enabled,omitempty" json:"enabled,omitempty"`

	// Alternatives is the ORDERED list of other pairings that serve THIS SAME
	// capability when the primary above cannot be used — a second provider for
	// the same tier of work, in the profile author's own order of preference.
	//
	// It is a list on the ASSIGNMENT rather than a change to Profile's value
	// type because every existing reader of `prof[capability]` keeps seeing the
	// PRIMARY and keeps working unchanged: a matrix with no `alternatives:`
	// anywhere resolves exactly as it did before this field existed, and the
	// walk simply has nothing to walk.
	//
	// WHAT MAKES A CANDIDATE UNUSABLE, and therefore advances the walk, is
	// listed and implemented in alternatives.go. Every one of them is judged
	// for the CANDIDATE with the candidate's OWN capacity — health does not
	// transfer across providers, which is the same rule applyShift already
	// follows for a mode shift's landing provider.
	//
	// AN ALTERNATIVE MAY NOT CARRY ALTERNATIVES OF ITS OWN. validate() reports
	// nesting as an Issue and the walk never descends: "try this, then this,
	// then this" is a flat ordered list, and a tree of fallbacks would make the
	// order the file states unreadable.
	//
	// A YAML SEQUENCE REPLACES WHOLESALE UNDER THE DEEP MERGE. deepMerge
	// recurses into maps only, so a user file that mentions `alternatives:` at
	// all replaces the whole shipped list for that capability rather than
	// merging element by element — write the complete list you want. The
	// primary's own provider/model/effort/fresh/enabled keys stay individually
	// editable exactly as they are today, because they are siblings of this
	// key rather than members of it.
	Alternatives []Assignment `yaml:"alternatives,omitempty" json:"alternatives,omitempty"`
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

// Pacing is the WINDOW-PROGRESS rule: how much of the allowance should be gone
// by now, and what to do when more of it is.
//
// It is a separate block from `health:` because it answers a different
// question. Health is a level — 40% used is 40% used. Pace is a level against
// the clock, and the same 40% is comfortable six days into a seven-day window
// and is a fleet about to run dry six hours into one. The two are combined
// rather than merged: pace may ADD conserve and may BLOCK spend-down, and it
// can never talk a RED or EXHAUSTED provider back down, because a provider that
// is nearly out is nearly out however elegantly it got there.
//
// EVERY VALUE HERE IS READ. `enabled: false` is the switch that reproduces the
// pre-pacing answers exactly, the two ratios are the bands, `bootstrap:` is
// what stops the first minutes of a window reading as a crisis, and
// `seven_day:` is the shape of the week. See limits/pace.go for the arithmetic
// and docs/limit-aware-routing.md for the prose.
type Pacing struct {
	// Enabled is a POINTER for the same reason every other flag in this file
	// is: a deep merge cannot delete, so switching pacing off has to be
	// something a user WRITES. Absent means enabled, matching the shipped file.
	Enabled *bool `yaml:"enabled,omitempty" json:"enabled,omitempty"`
	// ConserveAtRatio is the ratio (consumed / expected-by-now) at or above
	// which the mode becomes CONSERVE.
	ConserveAtRatio float64 `yaml:"conserve_at_ratio" json:"conserveAtRatio"`
	// BlockSpendDownAtRatio is where being ahead of the curve stops licensing
	// spend-down. Never above ConserveAtRatio; validate() says so.
	BlockSpendDownAtRatio float64         `yaml:"block_spend_down_at_ratio" json:"blockSpendDownAtRatio"`
	Bootstrap             PacingBootstrap `yaml:"bootstrap" json:"bootstrap"`
	SevenDay              SevenDayPacing  `yaml:"seven_day" json:"sevenDay"`
}

// IsEnabled reports whether pacing is in force. Absent means yes.
func (p Pacing) IsEnabled() bool { return p.Enabled == nil || *p.Enabled }

// PacingBootstrap is the start-of-window guard. Without it, one percent used
// against a fifth of a percent elapsed is a ratio of five, and every window
// would open in CONSERVE for its first few minutes.
type PacingBootstrap struct {
	// MinElapsedPct is the floor: below this share of the window elapsed, no
	// pace verdict is taken at all.
	MinElapsedPct float64 `yaml:"min_elapsed_pct" json:"minElapsedPct"`
	// ExpectedOffsetPct is added, in percentage points, to the expected share
	// before the division — so the denominator is never nearly zero.
	ExpectedOffsetPct float64 `yaml:"expected_offset_pct" json:"expectedOffsetPct"`
}

// SevenDayPacing is the shape of the week, and it applies to the seven-day
// window only: a five-hour window has no weekday shape to have an opinion
// about.
type SevenDayPacing struct {
	// Curve is `calendar` (linear in wall-clock time) or `workdays` (weekend
	// hours weighted down). Calendar ships, because a fleet that works at the
	// weekend would otherwise be told to conserve on Saturday for no reason.
	Curve string `yaml:"curve" json:"curve"`
	// Timezone is `local` (the host's own) or an IANA name. A weekend is a
	// local fact: a curve computed in UTC for a fleet in UTC+13 is wrong by
	// most of a day.
	Timezone string `yaml:"timezone" json:"timezone"`
	// WeekendWeight is one weekend hour's share of one weekday hour's budget.
	// It must be strictly positive — see validate() and limits.PaceConfig.
	WeekendWeight float64 `yaml:"weekend_weight" json:"weekendWeight"`
	// Weekend is what the weekend is for: `spend_tail` (nothing held back) or
	// `reserve` (WeekendReservePct of the allowance kept against the curve).
	Weekend string `yaml:"weekend" json:"weekend"`
	// WeekendReservePct is the held-back share under `reserve`. It is IGNORED
	// under `spend_tail`, and the explanation says so rather than leaving a
	// number in a file that changes nothing.
	WeekendReservePct float64 `yaml:"weekend_reserve_pct" json:"weekendReservePct"`
}

// Thresholds are the mode rules, kept as configuration rather than as constants
// buried in Go.
type Thresholds struct {
	SpendDown SpendDown `yaml:"spend_down" json:"spendDown"`
	Health    Health    `yaml:"health" json:"health"`
	Pacing    Pacing    `yaml:"pacing" json:"pacing"`
}

// Modes is the manual override: "auto" defers to Thresholds.
type Modes struct {
	Global    string            `yaml:"global" json:"global"`
	Providers map[string]string `yaml:"providers" json:"providers"`
}

// ModeShift is everything one routing mode does, and it is TWO knobs on one
// axis rather than one: which capability a role gets, and how hard the model it
// lands on is asked to think.
//
// The role table is INLINE — `conserve: {scout: cheap}` is still written
// exactly that way — so every routing.yaml written before effort stepping
// existed parses unchanged, and the two reserved keys sit beside the roles
// rather than under a nested block nobody would find.
//
// WHY EFFORT IS A SEPARATE MOVE FROM CAPABILITY. A capability shift changes
// which MODEL runs the work; an effort step changes how much THINKING that
// model is asked to do before it answers. They cost differently and they
// degrade differently: dropping Sol to Terra changes the answer's character,
// while dropping Sol from `high` to `medium` trims reasoning tokens off the same
// model. The gentler move is the right first response to a window that is merely
// running ahead of its curve, and the harsher one is still there for a window
// that is actually scarce. See docs/limit-aware-routing.md.
type ModeShift struct {
	// Roles is role -> capability: the shift table this block has always been.
	Roles map[string]string `yaml:",inline" json:"roles,omitempty"`

	// EffortStep is a NOTCH COUNT on the provider's own effort ladder, applied
	// to the assignment the answer actually lands on. Negative steps down
	// (conserve ships -1), positive steps up (spend_down ships +1), and 0 —
	// the absent value — reproduces the pre-stepping answer exactly, reasons
	// included.
	//
	// It is a notch count rather than a level name because THE LADDERS ARE NOT
	// PORTABLE: claude runs low..max and codex stops at xhigh, so `medium` means
	// a different distance from the top on each. One step down is the same
	// instruction on both.
	EffortStep int `yaml:"effort_step,omitempty" json:"effortStep,omitempty"`

	// EffortStepCapabilities is the allow-list of capabilities this mode's step
	// applies to. Empty means EVERY capability, which is not what ships: the
	// default names the four tiers where thinking time is the expensive part
	// (frontier, frontier_max, deep_reviewer, frontier_plus), because a scout on
	// Sonnet at `medium` is a worse scout rather than a cheaper one — the
	// saving on a cheap tier is small and the loss is not.
	EffortStepCapabilities []string `yaml:"effort_step_capabilities,omitempty" json:"effortStepCapabilities,omitempty"`
}

// StepsCapability reports whether this mode's effort step applies to a
// capability. An empty allow-list means every capability.
func (s ModeShift) StepsCapability(capability string) bool {
	capability = strings.ToLower(strings.TrimSpace(capability))
	if len(s.EffortStepCapabilities) == 0 {
		return true
	}
	for _, c := range s.EffortStepCapabilities {
		if strings.ToLower(strings.TrimSpace(c)) == capability {
			return true
		}
	}
	return false
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

// UnrankedCapability is what RankOf reports for a capability the matrix's
// `capability_ranks:` block does not name.
const UnrankedCapability = -1

// UnrankedCapabilityStrength is how STRONG an unrankable capability is when a
// ceiling has to judge one anyway: stronger than anything the file can rank.
//
// It is not UnrankedCapability (-1) because these answer different questions.
// "Is this ranked?" is -1's job. "How much authority must I assume this confers
// when I cannot look it up?" has exactly one safe answer at a gate, and it is
// not zero. Kept far above any plausible hand-written rank so a matrix that
// ranks its ladder 1..10 cannot collide with it.
const UnrankedCapabilityStrength = 1 << 20

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
	Version       int      `yaml:"version" json:"version"`
	ActiveProfile string   `yaml:"active_profile" json:"activeProfile"`
	Capabilities  []string `yaml:"capabilities" json:"capabilities"`
	// CapabilityRanks orders the capability names by STRENGTH, which is the one
	// question `capabilities:` cannot answer.
	//
	// `capabilities:` is a VOCABULARY and its order is documentation, not a
	// ladder: `reviewer` is listed after `frontier` and resolves to Sonnet High,
	// which is cheaper than frontier's Sol High. Reading list position as
	// strength would make the shipped `default: {max_capability: frontier}`
	// ceiling refuse every reviewer and deep_reviewer spawn — a clamp firing on
	// the CHEAPER side of the thing it is protecting. So the ordering is stated,
	// in the file, as its own block, and ties are allowed because the axis
	// genuinely has them (deep_reviewer and frontier are the same strength on
	// two different model families).
	//
	// A capability with no rank cannot be compared, and the ceiling FAILS CLOSED
	// on it — see [Matrix.CheckSpawn]. validate() reports every unranked
	// capability at load so that is discovered when the file is saved rather
	// than when a spawn is refused.
	CapabilityRanks map[string]int      `yaml:"capability_ranks" json:"capabilityRanks"`
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
	ModeShifts map[string]ModeShift `yaml:"mode_shifts" json:"modeShifts"`
	Ceilings   map[string]Ceiling   `yaml:"ceilings" json:"ceilings"`

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
	// CatalogChecked reports whether ValidateAgainstCatalog has run against THIS
	// matrix yet, and it is here so that "no issues" can be told apart from "not
	// asked yet". The catalog half of validation is deferred off the boot path
	// (see Service.ValidateCatalog: at boot the bus that answers it is not
	// listening), so between a load and the next tick the model ids in Issues
	// are the pure ones only.
	CatalogChecked bool `yaml:"-" json:"catalogChecked"`

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

// ModeFor is the routing mode in force for a provider: its own entry when that
// entry states an opinion, otherwise the global one, otherwise "auto".
//
// A PER-PROVIDER `auto` IS NOT AN OVERRIDE. It is the absence of one, and
// reading it as an override made `modes.global` dead on arrival: the shipped
// file writes `providers: {codex: auto, claude: auto}` to document the shape, a
// deep merge cannot delete them, and a per-provider entry outranks the global —
// so a user who wrote `modes: {global: conserve}` and nothing else was overruled
// by two lines the hub itself had put in their file, for the only two providers
// that have a readable allowance. §34 offers `auto` as a value precisely so a
// provider can say "let the thresholds decide"; when a global is set, that
// sentence means "I have no opinion here", not "ignore what you just said
// globally". A provider that genuinely wants to opt out of a global conserve
// says `normal`, which is a verdict rather than a deferral, and which this
// function does honour.
func (m *Matrix) ModeFor(provider string) string {
	if v, ok := m.Modes.Providers[normalizeProvider(provider)]; ok && strings.TrimSpace(v) != "" {
		if parsed, ok := ParseMode(v); !ok || parsed != ModeAuto {
			return v
		}
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
	shift, ok := m.ModeShifts[strings.ToLower(strings.TrimSpace(mode))]
	if !ok {
		return "", false
	}
	c, ok := shift.Roles[role]
	if !ok || strings.TrimSpace(c) == "" {
		return "", false
	}
	return c, true
}

// RoutableProviders is every provider this matrix can actually send work to —
// the primaries and the alternatives of every profile, deduplicated and sorted.
//
// It exists so the live availability probe asks about the providers a decision
// could land on rather than about the whole `providers:` vocabulary: booting a
// CLI to find out whether a provider nothing routes to is installed is a cost
// with no reader.
func (m *Matrix) RoutableProviders() []string {
	if m == nil {
		return nil
	}
	seen := map[string]bool{}
	for _, prof := range m.Profiles {
		for _, a := range prof {
			for _, p := range append([]Assignment{a}, a.Alternatives...) {
				if id := normalizeProvider(p.Provider); id != "" {
					seen[id] = true
				}
			}
		}
	}
	return sortedKeys(seen)
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
// It is deliberately LEXICAL over an already-resolved path: filepath.Clean only
// normalizes separators and dot components here; this function does not resolve
// symlinks, and must never be handed a caller-supplied string directly. The
// enforcement site canonicalizes first —
// that is the same check-path/opened-path rule the filesystem guard is built on,
// and a ceiling looked up on an unresolved path is a ceiling a symlink walks
// around.
func (m *Matrix) CeilingFor(canonicalDir string) (Ceiling, string) {
	target := filepath.Clean(canonicalDir)
	bestKey := ""
	bestPath := ""
	for key := range m.Ceilings {
		if key == CeilingDefaultKey || !filepath.IsAbs(key) {
			continue
		}
		candidate := filepath.Clean(key)
		if !isAtOrInside(candidate, target) {
			continue
		}
		// Clean before comparing so a hand-written Windows key using forward
		// slashes matches the canonical backslash form. Break equal-depth ties
		// deterministically: maps deliberately do not promise iteration order.
		if len(candidate) > len(bestPath) || (len(candidate) == len(bestPath) && key < bestKey) {
			bestKey, bestPath = key, candidate
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
	if routingPathsEqual(target, dir) {
		return true
	}
	if !strings.HasSuffix(dir, string(filepath.Separator)) {
		dir += string(filepath.Separator)
	}
	return routingPathHasPrefix(target, dir)
}
