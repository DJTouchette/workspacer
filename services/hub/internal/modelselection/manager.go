package modelselection

import (
	"errors"
	"fmt"
	"regexp"
	"strings"
)

var (
	ErrInvalidManagerMap    = errors.New("invalid-manager-map")
	ErrInvalidManagerModel  = errors.New("invalid-manager-model")
	ErrInvalidManagerEffort = errors.New("invalid-manager-effort")
	ErrForeignManagerModel  = errors.New("foreign-manager-model")
)

var managerProviders = map[string]bool{
	"claude": true, "codex": true, "copilot": true, "opencode": true, "pi": true,
}

type ManagerPreferences struct {
	Models      map[string]string
	Efforts     map[string]string
	Contexts    map[string]*uint64
	ContextSet  map[string]bool
	ModelsSet   bool
	EffortsSet  bool
	ContextsSet bool
}

func managerMap(raw any, name string, strict bool) (map[string]any, bool, error) {
	if raw == nil {
		return nil, false, nil
	}
	m, ok := raw.(map[string]any)
	if !ok {
		if strict {
			return nil, true, fmt.Errorf("%w: agents.%s must be an object", ErrInvalidManagerMap, name)
		}
		return nil, true, nil
	}
	return m, true, nil
}

func managerWindow(raw any) (*uint64, bool) {
	if raw == nil {
		return nil, true
	}
	var n uint64
	switch value := raw.(type) {
	case int:
		if value <= 0 {
			return nil, false
		}
		n = uint64(value)
	case int64:
		if value <= 0 {
			return nil, false
		}
		n = uint64(value)
	case uint:
		n = uint64(value)
	case uint64:
		n = value
	case float64:
		if value <= 0 || value != float64(uint64(value)) {
			return nil, false
		}
		n = uint64(value)
	default:
		return nil, false
	}
	if n == 0 {
		return nil, false
	}
	return &n, true
}

// CanonicalManagerPreferences is the Go twin of main/shared/managerSelection.
// strict=true refuses malformed saves; strict=false is the compatibility read
// path and drops only invalid entries. Legacy Claude [1m] input is split into
// the canonical model identity and numeric context preference.
func CanonicalManagerPreferences(agents map[string]any, strict bool) (ManagerPreferences, error) {
	out := ManagerPreferences{
		Models: map[string]string{}, Efforts: map[string]string{},
		Contexts: map[string]*uint64{}, ContextSet: map[string]bool{},
	}
	if agents == nil {
		return out, nil
	}

	models, present, err := managerMap(agents["managerModels"], "managerModels", strict)
	if err != nil {
		return out, err
	}
	out.ModelsSet = present
	for key, raw := range models {
		model, ok := raw.(string)
		if !managerProviders[key] || !ok {
			if strict {
				return out, fmt.Errorf("%w: agents.managerModels.%s", ErrInvalidManagerModel, key)
			}
			continue
		}
		model = strings.TrimSpace(model)
		if model != "" && foreignManagerModel(key, model) {
			if strict {
				return out, fmt.Errorf("%w: agents.managerModels.%s", ErrForeignManagerModel, key)
			}
			continue
		}
		out.Models[key] = model
	}

	efforts, present, err := managerMap(agents["managerEfforts"], "managerEfforts", strict)
	if err != nil {
		return out, err
	}
	out.EffortsSet = present
	for key, raw := range efforts {
		effort, ok := raw.(string)
		if !managerProviders[key] || !ok {
			if strict {
				return out, fmt.Errorf("%w: agents.managerEfforts.%s", ErrInvalidManagerEffort, key)
			}
			continue
		}
		out.Efforts[key] = strings.TrimSpace(effort)
	}

	contexts, present, err := managerMap(agents["managerContextWindows"], "managerContextWindows", strict)
	if err != nil {
		return out, err
	}
	out.ContextsSet = present
	for key, raw := range contexts {
		if key != "claude" && key != "codex" {
			if strict {
				return out, fmt.Errorf("%w: agents.managerContextWindows.%s", ErrUnsupportedContextWindow, key)
			}
			continue
		}
		window, ok := managerWindow(raw)
		if !ok {
			if strict {
				return out, fmt.Errorf("%w: agents.managerContextWindows.%s", ErrInvalidContextWindow, key)
			}
			continue
		}
		out.ContextSet[key] = true
		out.Contexts[key] = window
	}

	if model := out.Models["claude"]; strings.TrimSpace(model) != "" {
		window := out.Contexts["claude"]
		selection, normalizeErr := Normalize(model, window)
		if normalizeErr == nil && selection.ContextWindow != nil && *selection.ContextWindow != 200_000 && *selection.ContextWindow != OneMillion {
			normalizeErr = fmt.Errorf("%w: Claude manager context must be a validated 200K or 1M model variant", ErrUnsupportedContextWindow)
		}
		if normalizeErr == nil && selection.ContextWindow != nil && *selection.ContextWindow == 200_000 && isClaudeInherentOneMillionModel(selection.Model) {
			normalizeErr = fmt.Errorf("%w: %s exposes only its inherent 1M context", ErrUnsupportedContextWindow, selection.Model)
		}
		if normalizeErr != nil {
			if strict {
				return out, normalizeErr
			}
			selection, normalizeErr = Normalize(model, nil)
			if normalizeErr != nil {
				delete(out.Models, "claude")
				delete(out.Contexts, "claude")
				delete(out.ContextSet, "claude")
				return out, nil
			}
		}
		out.Models["claude"] = selection.Model
		if out.ContextSet["claude"] || selection.ContextWindow != nil {
			out.ContextSet["claude"] = true
			out.Contexts["claude"] = cloneWindow(selection.ContextWindow)
		}
	} else if out.ContextSet["claude"] && out.Contexts["claude"] != nil {
		if strict {
			return out, fmt.Errorf("%w: Claude context requires a manager model", ErrInvalidContextWindow)
		}
		delete(out.Contexts, "claude")
		delete(out.ContextSet, "claude")
	}
	return out, nil
}

func (p ManagerPreferences) Context(provider string) (*uint64, bool) {
	if !p.ContextSet[provider] {
		return nil, false
	}
	return cloneWindow(p.Contexts[provider]), true
}

var slashModel = regexp.MustCompile(`^[\w.-]+/[\w.:-]+$`)

func modelOwners(model string) map[string]bool {
	m := strings.TrimSpace(model)
	lower := strings.ToLower(m)
	owners := map[string]bool{}
	if lower == "default" || lower == "haiku" || lower == "sonnet" || lower == "sonnet[1m]" || lower == "opus" || lower == "opusplan" || lower == "fable" || strings.HasPrefix(lower, "claude-") {
		owners["claude"] = true
	}
	if regexp.MustCompile(`^(gpt-|o\d|codex-|gpt\d)`).MatchString(lower) {
		owners["codex"] = true
	}
	if lower == "auto" {
		owners["copilot"] = true
	}
	if slashModel.MatchString(m) {
		owners["opencode"] = true
		owners["pi"] = true
	}
	return owners
}

func foreignManagerModel(provider, model string) bool {
	owners := modelOwners(model)
	return len(owners) > 0 && !owners[provider]
}
