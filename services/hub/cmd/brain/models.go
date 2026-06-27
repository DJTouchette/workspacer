package main

// claude.listModels — the model-picker data. The aliases resolve to the latest
// model of each family (so they track Claude Code updates with zero
// maintenance); the config-derived fields and the live `seen` list now come
// from the same sources the app uses: config.yaml plus the models observed in
// claudemon's live sessions. Mirrors claudeModels.ts.

import (
	"context"
	"encoding/json"
	"sort"
)

type modelAlias struct {
	Value string `json:"value"`
	Label string `json:"label"`
}

type listModelsResult struct {
	DefaultModel           string       `json:"defaultModel"`
	SkipPermissionsDefault bool         `json:"skipPermissionsDefault"`
	Aliases                []modelAlias `json:"aliases"`
	Seen                   []string     `json:"seen"`
}

func (r *registry) listModels(ctx context.Context) listModelsResult {
	cfg := r.cfg.get()
	claude, _ := cfg["claude"].(map[string]any)

	defaultModel, _ := claude["defaultModel"].(string)
	skip, _ := claude["skipPermissionsDefault"].(bool)

	seen := map[string]struct{}{}
	for _, m := range toStringSlice(claude["seenModels"]) {
		seen[m] = struct{}{}
	}
	for _, m := range r.liveModels(ctx) {
		seen[m] = struct{}{}
	}
	out := make([]string, 0, len(seen))
	for m := range seen {
		out = append(out, m)
	}
	sort.Strings(out)

	return listModelsResult{
		DefaultModel:           defaultModel,
		SkipPermissionsDefault: skip,
		Aliases: []modelAlias{
			{Value: "fable", Label: "Fable — latest"},
			{Value: "opus", Label: "Opus — latest"},
			{Value: "sonnet", Label: "Sonnet — latest"},
			{Value: "haiku", Label: "Haiku — latest"},
		},
		Seen: out,
	}
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
