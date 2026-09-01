// Package modelselection owns model identity/window normalization for the Go
// stack. Legacy suffixes are accepted only at ingress and Claude's marker is
// reconstructed only by ClaudeArgvModel at the external CLI boundary.
package modelselection

import (
	"errors"
	"fmt"
	"strings"
	"unicode"
)

const OneMillion uint64 = 1_000_000

// DefaultCodexContextWindow is the fresh-Codex spawn request from
// contracts/model-context-windows.json's providerContextDefaults block. Its
// contract test keeps this Go ingress aligned with the desktop and daemon.
const DefaultCodexContextWindow uint64 = 1_000_000

// ContextWindowForNewSpawn applies the provider request policy before model
// normalization. Codex permits an explicit context request without a model, so
// this must not be hidden behind a model-selection gate. Resumes deliberately
// stay untouched: sessions created before this feature have no durable request
// and must retain their provider default rather than be retroactively upgraded.
func ContextWindowForNewSpawn(provider string, contextWindow *uint64, isResume bool) *uint64 {
	if contextWindow != nil || isResume || !strings.EqualFold(strings.TrimSpace(provider), "codex") {
		return contextWindow
	}
	window := DefaultCodexContextWindow
	return &window
}

var (
	ErrEmptyModel               = errors.New("empty-model")
	ErrInvalidModelIdentity     = errors.New("invalid-model-identity")
	ErrInvalidContextWindow     = errors.New("invalid-context-window")
	ErrUnsupportedContextWindow = errors.New("unsupported-context-window")
	ErrConflictingContextWindow = errors.New("conflicting-context-window")
	ErrConflictingModelIdentity = errors.New("conflicting-model-identity")
)

type Selection struct {
	Model         string  `json:"model"`
	ContextWindow *uint64 `json:"contextWindow"`
}

type Resolved struct {
	Selection   Selection
	LegacyModel string
}

// ResolveInput consumes the additive wire shared by spawn and managed model
// switch. A canonical pair wins when present, while LegacyModel keeps old
// peers executable. Both generations must agree when both are sent.
//
// Claude alone owns [1m]/-1m syntax. Other providers' ids are opaque, so a
// non-Claude id ending in -1m survives byte-for-byte.
func ResolveInput(provider, legacyModel, modelIdentity string, contextWindow *uint64) (*Resolved, error) {
	if strings.TrimSpace(legacyModel) != "" && strings.IndexFunc(legacyModel, unsafeModelIdentityRune) >= 0 ||
		strings.TrimSpace(modelIdentity) != "" && strings.IndexFunc(modelIdentity, unsafeModelIdentityRune) >= 0 {
		return nil, ErrInvalidModelIdentity
	}
	legacy := strings.TrimSpace(legacyModel)
	identity := strings.TrimSpace(modelIdentity)
	hasCanonical := identity != "" || contextWindow != nil

	normalizedProvider := strings.ToLower(strings.TrimSpace(provider))
	if contextWindow != nil && normalizedProvider != "claude" && normalizedProvider != "codex" {
		return nil, ErrUnsupportedContextWindow
	}
	if normalizedProvider != "claude" {
		if contextWindow != nil && *contextWindow == 0 {
			return nil, ErrInvalidContextWindow
		}
		if hasCanonical && identity == "" && legacy == "" && normalizedProvider != "codex" {
			return nil, ErrEmptyModel
		}
		if identity != "" && legacy != "" && identity != legacy {
			return nil, ErrConflictingModelIdentity
		}
		model := identity
		if model == "" {
			model = legacy
		}
		if model == "" {
			return nil, nil
		}
		if legacy == "" {
			legacy = model
		}
		return &Resolved{Selection: Selection{Model: model, ContextWindow: cloneWindow(contextWindow)}, LegacyModel: legacy}, nil
	}

	var selection Selection
	var err error
	if hasCanonical {
		if identity == "" && legacy == "" {
			return nil, ErrEmptyModel
		}
		canonicalIdentity := identity
		if canonicalIdentity == "" {
			canonicalIdentity = legacy
		}
		selection, err = Normalize(canonicalIdentity, contextWindow)
		if err != nil {
			return nil, err
		}
		if identity != "" && selection.Model != identity {
			return nil, ErrConflictingModelIdentity
		}
		if identity != "" && legacy != "" {
			companion, err := Normalize(legacy, nil)
			if err != nil {
				return nil, err
			}
			expectedLegacy, err := ClaudeArgvModel(selection)
			if err != nil {
				return nil, err
			}
			expectedCompanion, err := Normalize(expectedLegacy, nil)
			if err != nil {
				return nil, err
			}
			if companion.Model != expectedCompanion.Model || !selectionWindowsEqual(companion.ContextWindow, expectedCompanion.ContextWindow) {
				return nil, ErrConflictingModelIdentity
			}
		}
	} else if legacy != "" {
		selection, err = Normalize(legacy, nil)
		if err != nil {
			return nil, err
		}
	} else {
		return nil, nil
	}
	legacy, err = ClaudeArgvModel(selection)
	if err != nil {
		return nil, err
	}
	return &Resolved{Selection: selection, LegacyModel: legacy}, nil
}

// Normalize accepts only trailing [1m] and -1m as legacy syntax. Unknown
// identities otherwise survive byte-for-byte after outer whitespace trimming.
func unsafeModelIdentityRune(r rune) bool {
	return unicode.IsControl(r) || r == '\u2028' || r == '\u2029'
}

func Normalize(model string, contextWindow *uint64) (Selection, error) {
	identity := strings.TrimSpace(model)
	if identity == "" {
		return Selection{}, ErrEmptyModel
	}
	if strings.IndexFunc(model, unsafeModelIdentityRune) >= 0 {
		return Selection{}, ErrInvalidModelIdentity
	}

	var legacyWindow *uint64
	for {
		lower := strings.ToLower(identity)
		switch {
		case strings.HasSuffix(lower, "[1m]"):
			identity = strings.TrimRight(identity[:len(identity)-4], " \t\r\n")
		case strings.HasSuffix(lower, "-1m"):
			identity = strings.TrimRight(identity[:len(identity)-3], " \t\r\n")
		default:
			goto stripped
		}
		w := OneMillion
		legacyWindow = &w
	}

stripped:
	if identity == "" {
		return Selection{}, ErrEmptyModel
	}
	if contextWindow != nil && *contextWindow == 0 {
		return Selection{}, ErrInvalidContextWindow
	}
	if legacyWindow != nil && contextWindow != nil && *legacyWindow != *contextWindow {
		return Selection{}, fmt.Errorf("%w: marker selects %d but contextWindow selects %d", ErrConflictingContextWindow, *legacyWindow, *contextWindow)
	}
	if contextWindow == nil {
		contextWindow = legacyWindow
	}
	return Selection{Model: identity, ContextWindow: cloneWindow(contextWindow)}, nil
}

// ClaudeArgvModel is the one Go boundary allowed to emit [1m]. Normalizing
// first keeps transitional legacy callers idempotent.
func ClaudeArgvModel(selection Selection) (string, error) {
	normalized, err := Normalize(selection.Model, selection.ContextWindow)
	if err != nil {
		return "", err
	}
	if normalized.ContextWindow != nil && *normalized.ContextWindow == OneMillion && !isClaudeInherentOneMillionModel(normalized.Model) {
		return normalized.Model + "[1m]", nil
	}
	return normalized.Model, nil
}

// Fable and Mythos are inherently 1M; they are not marker-selectable variants.
// contracts/model-context-windows.json drives this boundary in all three
// languages so Go cannot invent an argv spelling the Claude catalog never emits.
func isClaudeInherentOneMillionModel(model string) bool {
	identity := strings.ToLower(model)
	return strings.Contains(identity, "fable") || strings.Contains(identity, "mythos")
}

func ErrorCode(err error) string {
	switch {
	case errors.Is(err, ErrEmptyModel):
		return ErrEmptyModel.Error()
	case errors.Is(err, ErrInvalidModelIdentity):
		return ErrInvalidModelIdentity.Error()
	case errors.Is(err, ErrInvalidContextWindow):
		return ErrInvalidContextWindow.Error()
	case errors.Is(err, ErrUnsupportedContextWindow):
		return ErrUnsupportedContextWindow.Error()
	case errors.Is(err, ErrConflictingContextWindow):
		return ErrConflictingContextWindow.Error()
	case errors.Is(err, ErrConflictingModelIdentity):
		return ErrConflictingModelIdentity.Error()
	default:
		return ""
	}
}

func selectionWindowsEqual(a, b *uint64) bool {
	return a == nil && b == nil || a != nil && b != nil && *a == *b
}

func cloneWindow(window *uint64) *uint64 {
	if window == nil {
		return nil
	}
	w := *window
	return &w
}
