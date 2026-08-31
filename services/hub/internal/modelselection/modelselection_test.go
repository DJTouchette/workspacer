package modelselection

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type contract struct {
	SelectionCases []struct {
		Name                  string  `json:"name"`
		Model                 string  `json:"model"`
		ContextWindow         *uint64 `json:"contextWindow"`
		ExpectedModel         *string `json:"expectedModel"`
		ExpectedContextWindow *uint64 `json:"expectedContextWindow"`
		Error                 *string `json:"error"`
		Note                  string  `json:"note"`
	} `json:"selectionCases"`
	ClaudeArgvCases []struct {
		Name          string  `json:"name"`
		Model         string  `json:"model"`
		ContextWindow *uint64 `json:"contextWindow"`
		Expected      *string `json:"expected"`
		Error         *string `json:"error"`
		Note          string  `json:"note"`
	} `json:"claudeArgvCases"`
}

func loadContract(t *testing.T) contract {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "contracts", "model-context-windows.json"))
	if err != nil {
		t.Fatal(err)
	}
	var c contract
	if err := json.Unmarshal(raw, &c); err != nil {
		t.Fatal(err)
	}
	if len(c.SelectionCases) < 10 || len(c.ClaudeArgvCases) < 4 {
		t.Fatal("the model-selection contract corpus was gutted")
	}
	return c
}

func TestSelectionCases(t *testing.T) {
	for _, tc := range loadContract(t).SelectionCases {
		got, err := Normalize(tc.Model, tc.ContextWindow)
		if tc.Error != nil {
			if ErrorCode(err) != *tc.Error {
				t.Errorf("%s: error %q, want %q — %s", tc.Name, ErrorCode(err), *tc.Error, tc.Note)
			}
			continue
		}
		if err != nil {
			t.Errorf("%s: %v — %s", tc.Name, err, tc.Note)
			continue
		}
		if tc.ExpectedModel == nil || got.Model != *tc.ExpectedModel || !sameWindow(got.ContextWindow, tc.ExpectedContextWindow) {
			t.Errorf("%s: got %#v, want model %v/window %v — %s", tc.Name, got, tc.ExpectedModel, tc.ExpectedContextWindow, tc.Note)
		}
		lower := strings.ToLower(got.Model)
		if strings.HasSuffix(lower, "[1m]") || strings.HasSuffix(lower, "-1m") {
			t.Errorf("%s: normalized output still has a suffix: %q", tc.Name, got.Model)
		}
		twice, err := Normalize(got.Model, got.ContextWindow)
		if err != nil || twice.Model != got.Model || !sameWindow(twice.ContextWindow, got.ContextWindow) {
			t.Errorf("%s: normalization is not idempotent: %#v then %#v (%v)", tc.Name, got, twice, err)
		}
	}
}

func TestClaudeArgvCases(t *testing.T) {
	for _, tc := range loadContract(t).ClaudeArgvCases {
		got, err := ClaudeArgvModel(Selection{Model: tc.Model, ContextWindow: tc.ContextWindow})
		if tc.Error != nil {
			if ErrorCode(err) != *tc.Error {
				t.Errorf("%s: error %q, want %q — %s", tc.Name, ErrorCode(err), *tc.Error, tc.Note)
			}
			continue
		}
		if err != nil || tc.Expected == nil || got != *tc.Expected {
			t.Errorf("%s: got %q (%v), want %v — %s", tc.Name, got, err, tc.Expected, tc.Note)
		}
	}
}

func sameWindow(a, b *uint64) bool {
	return a == nil && b == nil || a != nil && b != nil && *a == *b
}
