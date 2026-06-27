package main

// claude.listModels — the model-picker data. The aliases are the canonical part
// (they resolve to the latest model of each family, so they track Claude Code
// updates with zero maintenance) and mirror claudeModels.ts exactly.
//
// The app additionally folds in config-derived fields (defaultModel,
// skipPermissionsDefault, seenModels). Those live in config.yaml, which the
// brain doesn't own yet — so they're returned empty/false here and will be
// enriched once config.* moves into the brain. Callers that only need the
// alias list (the common case) get full parity today.

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

func listClaudeModels() listModelsResult {
	return listModelsResult{
		DefaultModel:           "",
		SkipPermissionsDefault: false,
		Aliases: []modelAlias{
			{Value: "fable", Label: "Fable — latest"},
			{Value: "opus", Label: "Opus — latest"},
			{Value: "sonnet", Label: "Sonnet — latest"},
			{Value: "haiku", Label: "Haiku — latest"},
		},
		Seen: []string{},
	}
}
