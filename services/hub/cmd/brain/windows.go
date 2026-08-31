package main

import "strings"

// The context window: one table, one resolver.
//
// TWINS: services/claudemon/src/session/windows.rs (Rust) and
// apps/desktop/src/main/services/modelContextWindows.ts (TS). All three are
// pinned to contracts/model-context-windows.json by windows_test.go and its
// opposite numbers — edit one table and the others go red.
//
// The brain's stake in this is the model picker: `claude.listModels` ships a
// `context` badge per alias ("200K" / "1M") that it and the desktop both used
// to hardcode, which made them a sixth opinion on a question four other tables
// were already answering differently. The badge now comes from this table, so
// a window that changes in the fixture changes the picker too.
//
// The other half is the shape of NOT KNOWING: a window is (uint64, bool) and
// `false` means "we do not know", which is a different fact from any number.
// `enrich.go` omits `contextLimit` entirely rather than map an unknown to a
// JSON null or a fake 200000.

// windowMatchKind is normative, not an implementation detail: the o3/o4 rows
// are prefix rows precisely so an `o3` buried inside an unrelated id does not
// match, and a port that read every row as a substring would pass every lookup
// case in the fixture except that one.
type windowMatchKind string

const (
	windowMatchContains windowMatchKind = "contains"
	windowMatchPrefix   windowMatchKind = "prefix"
	windowMatchSuffix   windowMatchKind = "suffix"
)

type windowRow struct {
	// Lowercased needle.
	Match  string
	Kind   windowMatchKind
	Window uint64
}

// contextWindows is THE TABLE, in order: first match wins, so the specific rows
// come before the families they overlap. Pinned row-for-row to the fixture's
// `windows` block.
var contextWindows = []windowRow{
	// Marker rows first — a statement about the WINDOW outranks a statement
	// about the family, whichever family carries it.
	{"[1m]", windowMatchSuffix, 1_000_000},
	{"-1m", windowMatchSuffix, 1_000_000},
	// 1M-native: the max window is also the default, so these ids never carry a
	// marker. Before the generic claude row or their gauges read 5x too full.
	{"fable", windowMatchContains, 1_000_000},
	{"mythos", windowMatchContains, 1_000_000},
	{"gemini", windowMatchContains, 1_048_576},
	{"gpt-4.1", windowMatchContains, 1_047_576},
	// Table KNOWLEDGE, not a fallback: an unmarked Claude model really does hold
	// 200k. The four places that spelled *unknown* 200000 are gone.
	{"claude", windowMatchContains, 200_000},
	{"gpt-5", windowMatchContains, 272_000},
	{"codex", windowMatchContains, 272_000},
	{"gpt-4o", windowMatchContains, 128_000},
	{"o3", windowMatchPrefix, 200_000},
	{"o4", windowMatchPrefix, 200_000},
	{"/o3", windowMatchContains, 200_000},
	{"/o4", windowMatchContains, 200_000},
	{"grok", windowMatchContains, 256_000},
	{"deepseek", windowMatchContains, 131_072},
	{"kimi", windowMatchContains, 262_144},
	{"qwen", windowMatchContains, 262_144},
}

// driftToleranceNum/Den: how far past the claimed window a session may be
// observed before we stop believing the claim. Two percent absorbs the
// provider's own rounding.
const (
	driftToleranceNum = 102
	driftToleranceDen = 100
)

// windowForModel is the table's answer for a concrete model id. The bool is
// false when no row covers it — the honest unknown, which every client already
// renders by hiding the meter rather than drawing a familiar-looking wrong one.
func windowForModel(model string) (uint64, bool) {
	m := strings.ToLower(model)
	for _, row := range contextWindows {
		var hit bool
		if row.Kind == windowMatchPrefix {
			hit = strings.HasPrefix(m, row.Match)
		} else if row.Kind == windowMatchSuffix {
			hit = strings.HasSuffix(m, row.Match)
		} else {
			hit = strings.Contains(m, row.Match)
		}
		if hit {
			return row.Window, true
		}
	}
	return 0, false
}

// requestedWindowFor is the narrower question: did the model string a session
// was ASKED for name a 1M window? Claude Code strips `[1m]` from the id it
// writes into the transcript, so the spawn request is the only carrier of that
// choice until the provider reports a window of its own.
//
// `false` means "says nothing", NOT "200k". A bare `opus` may be a 200k session
// or whatever Claude Code's default becomes tomorrow; pinning a number here is
// exactly how a wrong window gets asserted from token zero.
func requestedWindowFor(model string) (uint64, bool) {
	m := strings.ToLower(strings.TrimSpace(model))
	if strings.HasSuffix(m, "[1m]") || strings.HasSuffix(m, "-1m") ||
		strings.Contains(m, "fable") || strings.Contains(m, "mythos") {
		return 1_000_000, true
	}
	return 0, false
}

// windowSignals is everything beyond the transcript's model id that can speak
// to a session's window. Each field carries its own presence flag, because
// "not reported" has to stay distinguishable from zero.
type windowSignals struct {
	// The window the provider itself reported for THIS session.
	Reported    uint64
	HasReported bool
	// The user's ~/.workspacer/model-rates.json context_limit.
	Override    uint64
	HasOverride bool
	// The model string the session was asked for at spawn.
	RequestedModel string
	// The session's high-water context occupancy, for the drift alarm.
	PeakContext uint64
}

// resolveContextWindow is THE RESOLVER. One hierarchy, and the order is the
// design:
//
//  1. Reported — the window the provider gave for THIS session. A fact.
//  2. Override — the user's model-rates.json context_limit. They are overruling
//     us deliberately, so it outranks the marker below. This REPLACED a max(),
//     under which a coarse alias could silently raise the window back over what
//     the user wrote.
//  3. RequestedModel's [1m] marker — known from token zero, which is what makes
//     birth-time knowledge possible.
//  4. the contract table, by concrete model id.
//  5. unknown. Never 200000.
//
// Then the DRIFT ALARM: a claim the session has been observed to EXCEED (past
// the tolerance) is demonstrably wrong, so it is DISQUALIFIED and the next
// claim down the hierarchy is tried; unknown is the answer only when every
// claim has been disproved. This is the retrospective 200k->1M promotion,
// demoted from a source of truth to an alarm — it used to silently REWRITE the
// window to 1M, a guess dressed as a correction.
//
// Disqualify-and-continue rather than disqualify-and-stop, because stopping
// discarded claims the evidence never touched. Claude Code's own statusLine
// reports 200000 for a session spawned `opus[1m]`, so a live 1M worker holding
// 356k tokens had its REPORTED window disproved and then, instead of falling
// through to the `[1m]` marker that says 1M and that 356k does not contradict,
// resolved to unknown — which is how a real 1M worker came to draw a pegged
// 100% context bar. Falling through is not inventing a replacement: every
// candidate here already existed and was already ranked.
func resolveContextWindow(model string, sig windowSignals) (uint64, bool) {
	for _, claim := range windowClaims(model, sig) {
		if sig.PeakContext > claim*driftToleranceNum/driftToleranceDen {
			continue // disproved by this session's own occupancy
		}
		return claim, true
	}
	return 0, false
}

// windowClaims is the hierarchy, in order, as a list rather than a single
// answer — so the drift alarm can drop a disproved claim and keep going.
func windowClaims(model string, sig windowSignals) []uint64 {
	var out []uint64
	if sig.HasReported && sig.Reported > 0 {
		out = append(out, sig.Reported)
	}
	if sig.HasOverride && sig.Override > 0 {
		out = append(out, sig.Override)
	}
	if w, ok := requestedWindowFor(sig.RequestedModel); ok {
		out = append(out, w)
	}
	if w, ok := windowForModel(model); ok {
		out = append(out, w)
	}
	return out
}

// formatClaudeAliasWindow renders the model picker's `context` badge for one of
// Claude Code's own aliases. The alias rows in models.go (and their TypeScript
// twin in claudeModels.ts) used to hardcode these strings, which is how a
// display-only sixth window table came to exist.
//
// DOMAIN: a `claude.listModels` alias — `opus`, `sonnet[1m]`, `haiku`, `fable`,
// or a concrete Claude id the user has been seen running. Nothing else is ever
// passed here. That matters because a bare alias names no vendor and so matches
// no row: the table keys the family on the string "claude", which a concrete
// transcript id always carries and an alias never does. Hence the second
// lookup — within this domain, prepending the family is the identity, not a
// guess. A marker alias (`opus[1m]`) or a 1M-native one (`fable`) is answered
// by the first lookup and never reaches it.
func formatClaudeAliasWindow(alias string) string {
	w, ok := windowForModel(alias)
	if !ok {
		w, ok = windowForModel("claude-" + alias)
	}
	if !ok {
		// An id the table has never heard of gets no badge rather than an
		// invented one — the same honest unknown as everywhere else.
		return ""
	}
	switch {
	case w >= 1_000_000:
		return "1M"
	default:
		return formatThousands(w)
	}
}

func formatThousands(w uint64) string {
	k := w / 1000
	digits := ""
	for k > 0 {
		digits = string(rune('0'+k%10)) + digits
		k /= 10
	}
	if digits == "" {
		digits = "0"
	}
	return digits + "K"
}
