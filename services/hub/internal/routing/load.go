package routing

import (
	_ "embed"
	"fmt"
	"path/filepath"
	"reflect"
	"sort"
	"strings"

	yaml "gopkg.in/yaml.v3"
)

// defaultMatrixYAML is the SHIPPED DEFAULT, and it does two jobs.
//
//  1. It is the BASE OF THE MERGE. A user file that omits frontier_plus still
//     resolves it; a release that adds a role still works against a file written
//     before that role existed. Same pattern as cmd/brain's config_defaults.json,
//     minus the generated TypeScript twin — there is no TS side here.
//  2. Its bytes are what gets written to disk on first run, VERBATIM, comments
//     and all. That is why the header comment lives in the file rather than
//     being prepended at write time: a file the user is told to edit has to
//     exist, and it has to explain itself.
//
//go:embed routing.default.yaml
var defaultMatrixYAML []byte

// DefaultBytes is the shipped default document, for the seeder and for tests.
func DefaultBytes() []byte {
	out := make([]byte, len(defaultMatrixYAML))
	copy(out, defaultMatrixYAML)
	return out
}

// providerAliases maps the vendor names the design spec uses onto the provider
// ids the spawn wire actually takes. A matrix pasted out of the spec works, and
// says so in the log rather than silently naming a provider nothing can serve.
var providerAliases = map[string]string{
	"openai":    "codex",
	"anthropic": "claude",
}

// knownProviders is the spawn wire's provider vocabulary. A `provider:` outside
// this set is an Issue, not a refusal — the set grows, and an older hub must not
// reject a newer file outright.
var knownProviders = map[string]bool{
	"claude": true, "codex": true, "copilot": true, "opencode": true, "pi": true,
}

func normalizeProvider(p string) string {
	p = strings.ToLower(strings.TrimSpace(p))
	if alias, ok := providerAliases[p]; ok {
		return alias
	}
	return p
}

// CatalogModel is one model a provider can actually launch right now.
type CatalogModel struct {
	ID string
	// EffortLevels is the reasoning-effort ladder this model accepts, when the
	// provider reports one. Empty means "not reported" and effort goes
	// unvalidated — never "no efforts allowed".
	EffortLevels []string
}

// Catalog is the live model catalog, injected rather than dialled here: this
// package does no I/O, so it stays importable and testable with no server.
//
// The wiring supplies claudemon's `GET /providers/:provider/models` (which
// returns id/label/default/defaultEffort/effortLevels per harness by booting the
// CLI) and `claude.listModels` over the bus. An error means "could not ask" and
// the models for that provider go unvalidated — an unreachable daemon must not
// condemn a correct matrix.
type Catalog interface {
	Models(provider string) ([]CatalogModel, error)
}

// Defaults is the compiled-in matrix on its own, with no user file merged in.
// It is what every fallback in matrix.go resolves against.
func Defaults() (*Matrix, error) {
	doc, err := parseDoc(defaultMatrixYAML)
	if err != nil {
		return nil, fmt.Errorf("the compiled-in routing defaults do not parse: %w", err)
	}
	m, err := decodeMatrix(doc)
	if err != nil {
		return nil, fmt.Errorf("the compiled-in routing defaults do not decode: %w", err)
	}
	normalize(m)
	return m, nil
}

// Load merges a user document over the shipped defaults and validates the
// result.
//
// userYAML may be nil (nothing on disk) or may parse to an empty document (a
// 0-byte, whitespace-only or comment-only file); both mean "no overrides" and
// resolve to the shipped defaults. A document that does NOT parse is an error,
// and the caller's contract is to keep whatever matrix it already had.
//
// THE MERGE IS DEEP AND PER KEY. No block is wholesale. That is not a stylistic
// preference: contracts/wholesale-config-paths.json exists because a wholesale
// replace makes the value's TYPE load-bearing, and the two config writers
// disagreed about it in opposite directions — one coercing a malformed value to
// {} and deleting every project's settings while reporting a successful save.
// There is one writer here, but the lesson holds, and for a routing matrix
// "cannot delete" is the CORRECT failure mode.
func Load(source string, userYAML []byte) (*Matrix, error) {
	base, err := parseDoc(defaultMatrixYAML)
	if err != nil {
		return nil, fmt.Errorf("the compiled-in routing defaults do not parse: %w", err)
	}
	user, err := parseDoc(userYAML)
	if err != nil {
		return nil, err
	}
	// The spec's vendor names are folded onto workspacer provider ids BEFORE
	// the merge, not after the decode, and that ordering is load-bearing. The
	// shipped defaults always carry a `codex:` entry, so a user file that says
	// `openai:` produces a merged map holding BOTH spellings — and normalizing
	// afterwards means two keys collapsing onto one in Go map order, which is
	// randomized. That is a user's explicit `modes.providers.openai: conserve`
	// being honoured or silently discarded on a coin flip, per process start.
	// Renaming the key first makes the user's value overlay the default's the
	// ordinary way, and makes the load log name the key that actually applied.
	normalizeDocProviderKeys(user)
	applied, changed, unrecognized := keyPaths(user, base)
	m, err := decodeMatrix(deepMerge(base, user))
	if err != nil {
		return nil, err
	}
	normalize(m)
	if len(user) > 0 {
		m.Source = source
	}
	m.Applied, m.Changed, m.Unrecognized = applied, changed, unrecognized
	if fb, err := Defaults(); err == nil {
		m.fallback = fb
	}
	m.Issues = validate(m)
	return m, nil
}

// parseDoc turns YAML into a generic document. A nil/blank/comment-only input
// unmarshals to a nil map with NO error: that is not a parse failure, it is a
// document with no keys, and it must not be mistaken for one (the config writers
// learned that the expensive way — a 0-byte config.yaml that looked like a parse
// error would have had defaults written over the user's real file).
func parseDoc(raw []byte) (map[string]any, error) {
	if len(strings.TrimSpace(string(raw))) == 0 {
		return map[string]any{}, nil
	}
	var doc map[string]any
	if err := yaml.Unmarshal(raw, &doc); err != nil {
		return nil, err
	}
	if doc == nil {
		return map[string]any{}, nil
	}
	return doc, nil
}

// deepMerge overlays source onto a shallow copy of target: a null source value
// means "unset" (keep the default), nested maps recurse, and everything else
// (including sequences) replaces. Same object semantics as cmd/brain's
// deepMerge, pinned there by contracts/deepmerge-cases.json.
//
// The property that matters here is what it CANNOT do: no key of the base
// survives being absent from the source, so no user edit can delete a role, a
// capability, a profile or the default ceiling.
func deepMerge(target, source map[string]any) map[string]any {
	result := make(map[string]any, len(target))
	for k, v := range target {
		result[k] = v
	}
	for k, sv := range source {
		if sv == nil {
			continue
		}
		if svMap, ok := sv.(map[string]any); ok {
			if tvMap, ok := result[k].(map[string]any); ok {
				result[k] = deepMerge(tvMap, svMap)
				continue
			}
		}
		result[k] = sv
	}
	return result
}

// keyPaths walks the user's document and returns three lists of leaf key paths.
//
//	applied      every leaf the user's file carries. The literal answer to
//	             "what was taken from this file".
//	changed      the subset whose value DIFFERS from the shipped default. This
//	             is the one the load log names, and the reason is the seeded
//	             file: it starts out byte-identical to the defaults, so a user
//	             who edits one line has a file that "carries" a hundred keys,
//	             and enumerating all of them on every save buries the one that
//	             moved. A matrix that silently half-applied is what the log
//	             exists to prevent, and a key that restates the default cannot
//	             half-apply anything. The full list stays on the Matrix.
//	unrecognized leaves (and whole blocks) matching nothing in the shipped
//	             defaults — almost always a typo. Reported rather than refused:
//	             the schema grows, and an older hub rejecting a newer file
//	             outright would be worse than one ignoring a key it never knew.
func keyPaths(user, base map[string]any) (applied, changed, unrecognized []string) {
	var walk func(prefix string, u, b map[string]any)
	walk = func(prefix string, u, b map[string]any) {
		for k, v := range u {
			path := k
			if prefix != "" {
				path = prefix + "." + k
			}
			var bChild map[string]any
			bHas := false
			if b != nil {
				var bv any
				bv, bHas = b[k]
				bChild, _ = bv.(map[string]any)
			}
			if uChild, ok := v.(map[string]any); ok && len(uChild) > 0 {
				if !bHas {
					unrecognized = append(unrecognized, path)
				}
				walk(path, uChild, bChild)
				continue
			}
			applied = append(applied, path)
			switch {
			case !bHas:
				unrecognized = append(unrecognized, path)
				changed = append(changed, path)
			case !reflect.DeepEqual(b[k], v):
				changed = append(changed, path)
			}
		}
	}
	walk("", user, base)
	sort.Strings(applied)
	sort.Strings(changed)
	sort.Strings(unrecognized)
	return applied, changed, unrecognized
}

// decodeMatrix re-marshals the merged generic document and decodes it into the
// typed Matrix. Round-tripping through YAML rather than reflecting over the map
// keeps ONE schema definition — the struct tags — instead of a second hand-rolled
// decoder that would drift from it.
func decodeMatrix(doc map[string]any) (*Matrix, error) {
	raw, err := yaml.Marshal(doc)
	if err != nil {
		return nil, err
	}
	var m Matrix
	if err := yaml.Unmarshal(raw, &m); err != nil {
		return nil, err
	}
	return &m, nil
}

// normalizeDocProviderKeys renames alias-spelled provider KEYS in a parsed
// document, in the two blocks that are keyed by provider.
//
// A document that spells the same provider both ways is its own contradiction,
// and it is resolved deterministically rather than by map order: the canonical
// spelling wins key by key, and whatever the alias said that the canonical did
// not is merged underneath it. Nothing is dropped in silence.
func normalizeDocProviderKeys(doc map[string]any) {
	if doc == nil {
		return
	}
	if providers, ok := doc["providers"].(map[string]any); ok {
		doc["providers"] = renameAliasKeys(providers)
	}
	if modes, ok := doc["modes"].(map[string]any); ok {
		if providers, ok := modes["providers"].(map[string]any); ok {
			modes["providers"] = renameAliasKeys(providers)
		}
	}
}

func renameAliasKeys(in map[string]any) map[string]any {
	out := make(map[string]any, len(in))
	// Canonical spellings first, so an alias can only ever fill a gap.
	for _, k := range sortedKeys(in) {
		if normalizeProvider(k) == k {
			out[k] = in[k]
		}
	}
	for _, k := range sortedKeys(in) {
		canonical := normalizeProvider(k)
		if canonical == k {
			continue
		}
		existing, taken := out[canonical]
		if !taken {
			out[canonical] = in[k]
			continue
		}
		eMap, eOK := existing.(map[string]any)
		aMap, aOK := in[k].(map[string]any)
		if eOK && aOK {
			out[canonical] = deepMerge(aMap, eMap)
		}
	}
	return out
}

// normalize folds the spec's vendor names onto workspacer provider ids,
// everywhere a provider can be named.
func normalize(m *Matrix) {
	for pname, prof := range m.Profiles {
		for cap, a := range prof {
			changed := false
			if n := normalizeProvider(a.Provider); n != a.Provider {
				a.Provider, changed = n, true
			}
			// The ALTERNATIVES too, and for the same reason the primary is
			// folded: a list pasted out of the spec spells `anthropic`, and a
			// candidate whose provider never normalizes is one every
			// provider-keyed lookup below (health, `modes:`, `enabled:`) misses
			// in silence.
			for i, alt := range a.Alternatives {
				if n := normalizeProvider(alt.Provider); n != alt.Provider {
					a.Alternatives[i].Provider, changed = n, true
				}
			}
			if changed {
				prof[cap] = a
			}
		}
		m.Profiles[pname] = prof
	}
	m.Providers = normalizeKeys(m.Providers)
	m.Modes.Providers = normalizeKeys(m.Modes.Providers)
}

func normalizeKeys[V any](in map[string]V) map[string]V {
	if in == nil {
		return nil
	}
	out := make(map[string]V, len(in))
	for k, v := range in {
		out[normalizeProvider(k)] = v
	}
	return out
}

// validate reports what is wrong WITHOUT refusing the document. Every finding is
// something an operator wants to hear at load rather than discover at spawn
// time; none of them is a reason to run on a stale matrix, because the fallbacks
// in matrix.go already keep every role answerable.
func validate(m *Matrix) []Issue {
	var issues []Issue
	add := func(where, format string, args ...any) {
		issues = append(issues, Issue{Where: where, Detail: fmt.Sprintf(format, args...)})
	}

	if m.Version <= 0 {
		add("version", "missing or not a positive integer")
	}
	if _, ok := m.Profiles[m.ActiveProfile]; !ok {
		add("active_profile", "%q is not one of the profiles in this file — falling back to the shipped default", m.ActiveProfile)
	}

	declared := map[string]bool{}
	for _, c := range m.Capabilities {
		declared[c] = true
	}
	for role, capability := range m.Roles {
		if capability == "" {
			add("roles."+role, "no capability named")
			continue
		}
		if !declared[capability] {
			add("roles."+role, "capability %q is not in the `capabilities:` list", capability)
		}
	}
	for pname, prof := range m.Profiles {
		for _, c := range m.Capabilities {
			if _, ok := prof[c]; !ok {
				add("profiles."+pname, "does not resolve capability %q", c)
			}
		}
		for capability, a := range prof {
			where := "profiles." + pname + "." + capability
			if !declared[capability] {
				add(where, "not in the `capabilities:` list")
			}
			if strings.TrimSpace(a.Model) == "" {
				add(where, "no model named")
			}
			if !knownProviders[a.Provider] {
				add(where, "provider %q is not a workspacer provider id (%s)", a.Provider, providerList())
			}
			// EVERY ALTERNATIVE IS CHECKED THE SAME WAY THE PRIMARY IS, and the
			// path names the index so an operator can find the row. Capability
			// membership is NOT re-checked: an alternative inherits its parent's
			// capability by construction and has no name of its own to be
			// wrong. What it can be wrong about is a model, a provider id, and
			// nesting — and an alternative nobody validated is a fallover that
			// only fails when the primary is already unusable, which is the
			// worst moment to discover it. The walk itself refuses to route to
			// a candidate flagged here; see alternatives.go.
			for i, alt := range a.Alternatives {
				aw := alternativePath(pname, capability, i)
				if strings.TrimSpace(alt.Model) == "" {
					add(aw, "no model named")
				}
				if !knownProviders[alt.Provider] {
					add(aw, "provider %q is not a workspacer provider id (%s)", alt.Provider, providerList())
				}
				if len(alt.Alternatives) > 0 {
					add(aw, "carries `alternatives:` of its own — an alternative may not nest, because the fallover order is the flat list the file states and a tree of them is unreadable. Move them up beside %s's own alternatives, in the order they should be tried", where)
				}
				// `fresh` ON AN ALTERNATIVE IS ADVISORY ONLY, and that is a
				// real asymmetry worth flagging rather than a detail: the spawn
				// gate's checkFresh resolves `fresh` through freshAssignment,
				// which reads only the ACTIVE profile's PRIMARY for a
				// capability (see fresh.go) — it never looks at an
				// alternative's own row. So a fallover to an alternative whose
				// `fresh:` disagrees with its primary's silently carries the
				// PRIMARY's answer, not the row that actually ran. The shipped
				// default always agrees on both sides; a hand-edited file that
				// does not is the asymmetry this Issue exists to surface,
				// because reaching it silently is exactly how a same-family
				// reviewer stops being independent on the one day the primary
				// is down.
				if alt.Fresh != a.Fresh {
					add(aw, "fresh: %v disagrees with the primary's fresh: %v — a spawn gate resolves `fresh` from the PRIMARY only (freshAssignment in fresh.go never reads an alternative's own row), so this alternative's `fresh:` is advisory and the primary's flag is what actually governs a fallover onto it", alt.Fresh, a.Fresh)
				}
			}
		}
	}
	for p := range m.Providers {
		if !knownProviders[p] {
			add("providers."+p, "not a workspacer provider id (%s)", providerList())
		}
	}
	for _, mode := range sortedKeys(m.ModeShifts) {
		switch mode {
		case "conserve", "spend_down":
		case "normal", "auto":
			add("mode_shifts."+mode, "%q is not a shift: the `roles:` table is the normal answer and `auto` is not a mode at all, so entries here never apply", mode)
		default:
			add("mode_shifts."+mode, "%q is not a routing mode (conserve, spend_down)", mode)
		}
		for _, role := range sortedKeys(m.ModeShifts[mode].Roles) {
			where := "mode_shifts." + mode + "." + role
			if _, ok := m.Roles[role]; !ok {
				add(where, "no role %q in the `roles:` table, so this shift can never fire", role)
			}
			if c := m.ModeShifts[mode].Roles[role]; !declared[c] {
				add(where, "capability %q is not in the `capabilities:` list", c)
			}
		}
	}
	// The capability LADDER. `capabilities:` says which names exist; this says
	// which is stronger, and the ceiling cannot compare anything without it. Both
	// directions are reported: a capability with no rank is one every ceiling
	// clamps (CheckSpawn fails closed on it), and a rank for a capability that
	// does not exist is a typo whose ceiling comparison will never fire.
	for _, c := range m.Capabilities {
		if _, ok := m.CapabilityRanks[c]; !ok {
			add("capability_ranks", "capability %q has no rank, so a `ceilings:` entry cannot tell whether it is above or below the limit — every spawn declaring it is clamped until it is ranked", c)
		}
	}
	for _, name := range sortedKeys(m.CapabilityRanks) {
		if !declared[name] {
			add("capability_ranks."+name, "not in the `capabilities:` list, so nothing can ever be ranked by it")
		}
	}

	for _, key := range sortedKeys(m.Ceilings) {
		c := m.Ceilings[key]
		if key != CeilingDefaultKey && !filepath.IsAbs(key) {
			add("ceilings."+key, "ceiling key %q is not an absolute path on this platform — CeilingFor ignores non-absolute keys, so this row would silently fall through to a weaker ancestor or the default ceiling", key)
		}
		if c.MaxCapability != "" && !declared[c.MaxCapability] {
			add("ceilings."+key, "max_capability %q is not in the `capabilities:` list", c.MaxCapability)
		}
		if c.MaxCapability != "" && m.RankOf(c.MaxCapability) == UnrankedCapability {
			add("ceilings."+key, "max_capability %q has no `capability_ranks:` entry, so this ceiling compares nothing and clamps nothing", c.MaxCapability)
		}
		switch c.MaxToolScope {
		case "", "view", "triage", "operator":
		default:
			add("ceilings."+key, "max_tool_scope %q is not an authority tier (view, triage, operator)", c.MaxToolScope)
		}
	}
	if _, ok := m.Ceilings[CeilingDefaultKey]; !ok {
		add("ceilings", "no %q entry — a directory with no entry of its own would have no ceiling", CeilingDefaultKey)
	}
	// The pacing block, whose validation is long enough to live beside the code
	// that reads it (pacing.go) rather than in the middle of this function.
	issues = append(issues, validatePacing(m.Thresholds.Pacing)...)
	// The effort-stepping knobs, beside their own reader for the same reason:
	// `mode_shifts.<mode>.effort_step`, its capability allow-list, and every
	// row's `min_effort` floor. See effort.go.
	issues = append(issues, validateEffortStepping(m)...)
	return issues
}

// ValidateAgainstCatalog checks every model id in every profile against what the
// installed CLIs actually serve, and is separate from validate() because it does
// I/O through the injected Catalog and validate() must stay pure.
//
// A provider the catalog cannot answer for is SKIPPED, not condemned: a codex
// CLI that is not installed on this machine says nothing about whether the
// matrix is right. An empty-but-successful answer is treated the same way — the
// route documents an empty list as valid (Pi with no authed providers), and
// failing every model against it would be a false alarm on the one shape that
// means "I do not know".
func ValidateAgainstCatalog(m *Matrix, cat Catalog) []Issue {
	if cat == nil || m == nil {
		return nil
	}
	type known struct {
		ids     map[string]bool
		efforts map[string][]string
	}
	seen := map[string]*known{}
	lookup := func(provider string) *known {
		if k, ok := seen[provider]; ok {
			return k
		}
		models, err := cat.Models(provider)
		if err != nil || len(models) == 0 {
			seen[provider] = nil
			return nil
		}
		k := &known{ids: map[string]bool{}, efforts: map[string][]string{}}
		for _, mm := range models {
			id := strings.ToLower(strings.TrimSpace(mm.ID))
			k.ids[id] = true
			k.efforts[id] = mm.EffortLevels
		}
		seen[provider] = k
		return k
	}

	check := func(a Assignment, where string) []Issue {
		k := lookup(a.Provider)
		if k == nil {
			return nil
		}
		id := strings.ToLower(strings.TrimSpace(a.Model))
		if !k.ids[id] {
			return []Issue{{Where: where, Detail: fmt.Sprintf(
				"%s does not serve model %q — it offers %s", a.Provider, a.Model, strings.Join(sortedKeys(k.ids), ", "))}}
		}
		levels := k.efforts[id]
		if a.Effort == "" || len(levels) == 0 {
			return nil
		}
		if !containsFold(levels, a.Effort) {
			return []Issue{{Where: where, Detail: fmt.Sprintf(
				"%s %s does not take effort %q — it takes %s", a.Provider, a.Model, a.Effort, strings.Join(levels, ", "))}}
		}
		return nil
	}

	var issues []Issue
	for _, pname := range sortedKeys(m.Profiles) {
		prof := m.Profiles[pname]
		for _, capability := range sortedKeys(prof) {
			a := prof[capability]
			issues = append(issues, check(a, "profiles."+pname+"."+capability)...)
			// THE ALTERNATIVES ARE CHECKED AGAINST THE SAME CATALOG, and this
			// is not tidiness: the fallover walk treats a candidate flagged
			// here as unusable and steps over it, so an unchecked alternative
			// is one the router would happily hand to a CLI that does not serve
			// that model — at the exact moment the primary has already failed.
			for i, alt := range a.Alternatives {
				issues = append(issues, check(alt, alternativePath(pname, capability, i))...)
			}
		}
	}
	return issues
}

// alternativePath is the dotted document path of one alternative, and it is the
// JOIN KEY between validation and routing: validate() and ValidateAgainstCatalog
// write Issues at this path, and the fallover walk refuses to route to a
// candidate that has one. Two spellings of it would silently disarm that.
func alternativePath(profile, capability string, index int) string {
	return fmt.Sprintf("profiles.%s.%s.alternatives[%d]", profile, capability, index)
}

func containsFold(hay []string, needle string) bool {
	for _, h := range hay {
		if strings.EqualFold(strings.TrimSpace(h), strings.TrimSpace(needle)) {
			return true
		}
	}
	return false
}

func providerList() string { return strings.Join(sortedKeys(knownProviders), ", ") }

func sortedKeys[V any](m map[string]V) []string {
	out := make([]string, 0, len(m))
	for k := range m {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}
