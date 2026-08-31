package main

// claude.listModels — the model-picker data, and a PARITY surface.
//
// claude.listModels is in catalogMethods(), so the brain is the DEFAULT answerer
// for every web / mobile / remote client while the desktop IPC path serves its
// own copy. The two used to answer a different contract: four aliases here
// versus six there (the 1M-context `opus[1m]` / `sonnet[1m]` ids were simply
// unreachable from the web picker), "Opus — latest" versus a version-stamped
// "Opus 4.5" plus a context badge, no defaultPermissionMode at all (so the
// remembered permission mode was lost on that path), and a `seen` list that
// still offered Claude Code's internal `<synthetic>` placeholder as a selectable
// model id. Both suites asserted their own answer and both were green.
//
// contracts/claude-model-catalog-cases.json is the fixture both are held to now.
// Mirrors claudeModels.ts rule for rule.

import (
	"context"
	"encoding/json"
	"regexp"
	"sort"
	"strconv"
	"strings"

	"github.com/djtouchette/workspacer-hub/internal/modelselection"
)

type modelAlias struct {
	Model string `json:"model"`
	// Legacy picker adapter; canonical identity for old value-only clients.
	Value         string `json:"value"`
	Label         string `json:"label"`
	ContextWindow uint64 `json:"contextWindow"`
	// Context-window badge, e.g. "200K" | "1M".
	Context string `json:"context,omitempty"`
}

type listModelsResult struct {
	DefaultModel           string  `json:"defaultModel"`
	ContextWindow          *uint64 `json:"contextWindow"`
	SkipPermissionsDefault bool    `json:"skipPermissionsDefault"`
	// Permission mode remembered from the last spawn ("" = provider default).
	DefaultPermissionMode string       `json:"defaultPermissionMode"`
	Aliases               []modelAlias `json:"aliases"`
	Seen                  []string     `json:"seen"`
}

// concreteModelID is claudeModels.ts's parseConcreteId regex, verbatim.
var concreteModelID = regexp.MustCompile(`^claude-([a-z]+)-(\d+(?:-\d+)*?)(?:-\d{6,})?$`)

// parseConcreteID splits a concrete model id into family + dotted version, e.g.
// "claude-opus-4-5-20251101" -> ("opus", "4.5"). ok false when it is not one.
func parseConcreteID(id string) (family, version string, ok bool) {
	selection, err := modelselection.Normalize(id, nil)
	if err != nil {
		return "", "", false
	}
	m := concreteModelID.FindStringSubmatch(selection.Model)
	if m == nil {
		return "", "", false
	}
	return m[1], strings.ReplaceAll(m[2], "-", "."), true
}

// newerVersion reports a > b over dotted numeric versions, padding the shorter.
func newerVersion(a, b string) bool {
	pa, pb := strings.Split(a, "."), strings.Split(b, ".")
	n := len(pa)
	if len(pb) > n {
		n = len(pb)
	}
	at := func(parts []string, i int) int {
		if i >= len(parts) {
			return 0
		}
		v, _ := strconv.Atoi(parts[i])
		return v
	}
	for i := 0; i < n; i++ {
		if d := at(pa, i) - at(pb, i); d != 0 {
			return d > 0
		}
	}
	return false
}

// buildListModels is the pure half, so the contract fixture can drive it without
// a config store or a claudemon.
func buildListModels(defaultModel string, contextWindow *uint64, skip bool, defaultPermissionMode string, persisted, live []string) listModelsResult {
	uniq := map[string]struct{}{}
	seenAll := []string{}
	for _, raw := range append(append([]string{}, persisted...), live...) {
		selection, err := modelselection.Normalize(raw, nil)
		if err != nil {
			continue
		}
		m := selection.Model
		// "<synthetic>" is Claude Code's placeholder model id on synthetic
		// transcript messages — telemetry noise, not a launchable model.
		if strings.HasPrefix(m, "<") {
			continue
		}
		if _, dup := uniq[m]; dup {
			continue
		}
		uniq[m] = struct{}{}
		seenAll = append(seenAll, m)
	}
	sort.Strings(seenAll)

	// Newest concrete version observed per family, used to version-label the
	// alias rows below.
	newest := map[string]string{}
	for _, id := range seenAll {
		family, version, ok := parseConcreteID(id)
		if !ok {
			continue
		}
		if cur, have := newest[family]; !have || newerVersion(version, cur) {
			newest[family] = version
		}
	}

	// An alias already stands for the newest model of its family, so a seen id at
	// that same version would render as a duplicate row — absorb it into the
	// alias (which carries its version in the label) and keep only older ids.
	seen := make([]string, 0, len(seenAll))
	for _, id := range seenAll {
		family, version, ok := parseConcreteID(id)
		if ok && newest[family] == version {
			continue
		}
		seen = append(seen, id)
	}

	label := func(family, base string) string {
		if v, ok := newest[family]; ok {
			return base + " " + v
		}
		return base
	}
	defaultSelection := modelselection.Selection{Model: "", ContextWindow: nil}
	if strings.TrimSpace(defaultModel) != "" {
		selection, err := modelselection.Normalize(defaultModel, contextWindow)
		if err == nil {
			defaultSelection = selection
		}
	}
	alias := func(legacyValue, aliasLabel string) modelAlias {
		selection, err := modelselection.Normalize(legacyValue, nil)
		if err != nil {
			panic("invalid built-in Claude alias: " + legacyValue)
		}
		window := selection.ContextWindow
		if window == nil {
			if resolved, ok := windowForModel(selection.Model); ok {
				window = &resolved
			} else if resolved, ok := windowForModel("claude-" + selection.Model); ok {
				window = &resolved
			}
		}
		if window == nil {
			panic("Claude alias has no context-window contract: " + legacyValue)
		}
		value, err := modelselection.ClaudeArgvModel(modelselection.Selection{
			Model: selection.Model, ContextWindow: window,
		})
		if err != nil {
			panic("invalid built-in Claude alias: " + legacyValue)
		}
		return modelAlias{
			Model: selection.Model, Value: value, Label: aliasLabel,
			ContextWindow: *window, Context: formatClaudeAliasWindow(legacyValue),
		}
	}

	return listModelsResult{
		DefaultModel:           defaultSelection.Model,
		ContextWindow:          defaultSelection.ContextWindow,
		SkipPermissionsDefault: skip,
		DefaultPermissionMode:  defaultPermissionMode,
		// The Context badge is LOOKED UP, never spelled here: these strings were a
		// display-only sixth window table, unpinned to the four numeric ones, and
		// formatClaudeAliasWindow reads the same contract every other window in
		// the repo now comes from.
		Aliases: []modelAlias{
			// Fable's 1M window is both its maximum AND its default — there is no
			// 200K mode to select, so it has no separate `[1m]` row.
			alias("fable", label("fable", "Fable")),
			alias("opus", label("opus", "Opus")),
			alias("opus[1m]", label("opus", "Opus")),
			alias("sonnet", label("sonnet", "Sonnet")),
			alias("sonnet[1m]", label("sonnet", "Sonnet")),
			alias("haiku", label("haiku", "Haiku")),
		},
		Seen: seen,
	}
}

func (r *registry) listModels(ctx context.Context) listModelsResult {
	cfg := r.cfg.get()
	claude, _ := cfg["claude"].(map[string]any)

	defaultModel, _ := claude["defaultModel"].(string)
	contextWindow, _ := configWindow(claude["contextWindow"])
	skip, _ := claude["skipPermissionsDefault"].(bool)
	mode, _ := claude["defaultPermissionMode"].(string)

	return buildListModels(defaultModel, contextWindow, skip, mode, toStringSlice(claude["seenModels"]), r.liveModels(ctx))
}

// liveModels pulls the concrete model ids out of claudemon's live sessions
// (session.usage.model). Best-effort: if claudemon is unreachable we just return
// the persisted list, so the picker still works headless.
func (r *registry) liveModels(ctx context.Context) []string {
	raw, err := r.cm.listSessions(ctx)
	if err != nil {
		return nil
	}
	var sessions []map[string]any
	if err := json.Unmarshal(raw, &sessions); err != nil {
		return nil
	}
	var out []string
	for _, s := range sessions {
		if usage, ok := s["usage"].(map[string]any); ok {
			if m, ok := usage["model"].(string); ok && m != "" {
				out = append(out, m)
			}
		}
	}
	return out
}

func toStringSlice(v any) []string {
	arr, ok := v.([]any)
	if !ok {
		return nil
	}
	out := make([]string, 0, len(arr))
	for _, e := range arr {
		if s, ok := e.(string); ok {
			out = append(out, s)
		}
	}
	return out
}
