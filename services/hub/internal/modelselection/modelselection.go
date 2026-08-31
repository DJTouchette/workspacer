// Package modelselection owns model identity/window normalization for the Go
// stack. Legacy suffixes are accepted only at ingress and Claude's marker is
// reconstructed only by ClaudeArgvModel at the external CLI boundary.
package modelselection

import (
	"errors"
	"fmt"
	"strings"
)

const OneMillion uint64 = 1_000_000

var (
	ErrEmptyModel               = errors.New("empty-model")
	ErrInvalidContextWindow     = errors.New("invalid-context-window")
	ErrConflictingContextWindow = errors.New("conflicting-context-window")
)

type Selection struct {
	Model         string  `json:"model"`
	ContextWindow *uint64 `json:"contextWindow"`
}

// Normalize accepts only trailing [1m] and -1m as legacy syntax. Unknown
// identities otherwise survive byte-for-byte after outer whitespace trimming.
func Normalize(model string, contextWindow *uint64) (Selection, error) {
	identity := strings.TrimSpace(model)
	if identity == "" {
		return Selection{}, ErrEmptyModel
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
	case errors.Is(err, ErrInvalidContextWindow):
		return ErrInvalidContextWindow.Error()
	case errors.Is(err, ErrConflictingContextWindow):
		return ErrConflictingContextWindow.Error()
	default:
		return ""
	}
}

func cloneWindow(window *uint64) *uint64 {
	if window == nil {
		return nil
	}
	w := *window
	return &w
}
