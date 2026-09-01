package modelselection

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

type contract struct {
	ProviderContextDefaults []struct {
		Provider           string `json:"provider"`
		FreshContextWindow uint64 `json:"freshContextWindow"`
		Note               string `json:"note"`
	} `json:"providerContextDefaults"`
	InputCases []struct {
		Name                  string  `json:"name"`
		Provider              string  `json:"provider"`
		Model                 *string `json:"model"`
		ModelIdentity         *string `json:"modelIdentity"`
		ContextWindow         *uint64 `json:"contextWindow"`
		ExpectedModel         *string `json:"expectedModel"`
		ExpectedContextWindow *uint64 `json:"expectedContextWindow"`
		ExpectedLegacyModel   *string `json:"expectedLegacyModel"`
		Error                 *string `json:"error"`
		Note                  string  `json:"note"`
	} `json:"inputCases"`
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
	if len(c.ProviderContextDefaults) != 1 || len(c.InputCases) < 12 || len(c.SelectionCases) < 10 || len(c.ClaudeArgvCases) < 4 {
		t.Fatal("the model-selection contract corpus was gutted")
	}
	return c
}

func TestProviderContextDefaultsFollowTheContract(t *testing.T) {
	c := loadContract(t)
	for _, tc := range c.ProviderContextDefaults {
		if tc.Provider != "codex" {
			t.Fatalf("unsupported provider context default %q", tc.Provider)
		}
		if DefaultCodexContextWindow != tc.FreshContextWindow {
			t.Fatalf("Codex default = %d, want contract %d — %s", DefaultCodexContextWindow, tc.FreshContextWindow, tc.Note)
		}
		got := ContextWindowForNewSpawn(tc.Provider, nil, false)
		if got == nil || *got != tc.FreshContextWindow {
			t.Fatalf("model-less fresh Codex request = %v, want %d — %s", got, tc.FreshContextWindow, tc.Note)
		}
		if got := ContextWindowForNewSpawn(tc.Provider, nil, true); got != nil {
			t.Fatalf("legacy Codex resume must remain provider-default, got %d", *got)
		}
	}
}

func TestInputCases(t *testing.T) {
	for _, tc := range loadContract(t).InputCases {
		stringValue := func(value *string) string {
			if value == nil {
				return ""
			}
			return *value
		}
		got, err := ResolveInput(
			tc.Provider,
			stringValue(tc.Model),
			stringValue(tc.ModelIdentity),
			tc.ContextWindow,
		)
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
		if tc.ExpectedModel == nil {
			if got != nil {
				t.Errorf("%s: got %#v, want no selection — %s", tc.Name, got, tc.Note)
			}
			continue
		}
		if got == nil || got.Selection.Model != *tc.ExpectedModel ||
			!sameWindow(got.Selection.ContextWindow, tc.ExpectedContextWindow) ||
			tc.ExpectedLegacyModel == nil || got.LegacyModel != *tc.ExpectedLegacyModel {
			t.Errorf("%s: got %#v, want model %v/window %v/legacy %v — %s", tc.Name, got, tc.ExpectedModel, tc.ExpectedContextWindow, tc.ExpectedLegacyModel, tc.Note)
		}
	}
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
