package modelselection

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

type managerPreferenceCase struct {
	Name             string             `json:"name"`
	Agents           map[string]any     `json:"agents"`
	ExpectedModels   map[string]string  `json:"expectedModels"`
	ExpectedEfforts  map[string]string  `json:"expectedEfforts"`
	ExpectedContexts map[string]*uint64 `json:"expectedContexts"`
	Error            *string            `json:"error"`
	Note             string             `json:"note"`
}

func managerPreferenceCases(t *testing.T) []managerPreferenceCase {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "..", "..", "..", "contracts", "model-context-windows.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Cases []managerPreferenceCase `json:"managerPreferenceCases"`
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.Cases) < 6 {
		t.Fatal("manager preference contract corpus was gutted")
	}
	return fixture.Cases
}

func managerErrorCode(err error) string {
	for _, candidate := range []error{
		ErrInvalidManagerMap, ErrInvalidManagerModel, ErrInvalidManagerEffort,
		ErrForeignManagerModel, ErrInvalidContextWindow, ErrUnsupportedContextWindow,
		ErrConflictingContextWindow,
	} {
		if errors.Is(err, candidate) {
			return candidate.Error()
		}
	}
	return ""
}

func TestManagerPreferenceCases(t *testing.T) {
	for _, tc := range managerPreferenceCases(t) {
		t.Run(tc.Name, func(t *testing.T) {
			got, err := CanonicalManagerPreferences(tc.Agents, true)
			if tc.Error != nil {
				if code := managerErrorCode(err); code != *tc.Error {
					t.Fatalf("error %q, want %q — %s", code, *tc.Error, tc.Note)
				}
				return
			}
			if err != nil {
				t.Fatalf("%v — %s", err, tc.Note)
			}
			if !stringMapsEqual(got.Models, tc.ExpectedModels) {
				t.Errorf("models %#v, want %#v", got.Models, tc.ExpectedModels)
			}
			if !stringMapsEqual(got.Efforts, tc.ExpectedEfforts) {
				t.Errorf("efforts %#v, want %#v", got.Efforts, tc.ExpectedEfforts)
			}
			if len(got.ContextSet) != len(tc.ExpectedContexts) {
				t.Fatalf("contexts %#v, want %#v", got.Contexts, tc.ExpectedContexts)
			}
			for provider, want := range tc.ExpectedContexts {
				value, set := got.Context(provider)
				if !set || !sameWindow(value, want) {
					t.Errorf("context %s=%v/%v, want %v", provider, value, set, want)
				}
			}
		})
	}
}

type managerVocabularyOwnershipCase struct {
	Name   string   `json:"name"`
	Model  string   `json:"model"`
	Owners []string `json:"owners"`
	Note   string   `json:"note"`
}

func managerVocabularyOwnershipCases(t *testing.T) []managerVocabularyOwnershipCase {
	t.Helper()
	raw, err := sweepguard.ReadRepoFile("contracts", "model-vocabulary-ownership-cases.json")
	if err != nil {
		if errors.Is(err, sweepguard.ErrNoCheckout) {
			t.Skipf("not a monorepo checkout, so this cross-repo cross-check has nothing to read: %v", err)
		}
		t.Fatal(err)
	}
	var fixture struct {
		Cases []managerVocabularyOwnershipCase `json:"ownershipCases"`
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	if len(fixture.Cases) < 20 {
		t.Fatal("manager model vocabulary ownership contract corpus was gutted")
	}
	return fixture.Cases
}

func TestManagerModelVocabularyOwnershipCases(t *testing.T) {
	for _, tc := range managerVocabularyOwnershipCases(t) {
		t.Run(tc.Name, func(t *testing.T) {
			wantOwners := map[string]bool{}
			for _, provider := range tc.Owners {
				wantOwners[provider] = true
			}
			if got := modelOwners(tc.Model); !boolMapsEqual(got, wantOwners) {
				t.Fatalf("owners %v, want %v — %s", got, wantOwners, tc.Note)
			}

			for _, provider := range []string{"claude", "codex", "copilot", "opencode", "pi"} {
				wantForeign := len(wantOwners) > 0 && !wantOwners[provider]
				if got := foreignManagerModel(provider, tc.Model); got != wantForeign {
					t.Errorf("foreign(%q, %q) = %t, want %t — %s", provider, tc.Model, got, wantForeign, tc.Note)
				}
			}
		})
	}
}

func stringMapsEqual(a, b map[string]string) bool {
	if len(a) != len(b) {
		return false
	}
	for key, value := range a {
		if b[key] != value {
			return false
		}
	}
	return true
}

func boolMapsEqual(a, b map[string]bool) bool {
	if len(a) != len(b) {
		return false
	}
	for key, value := range a {
		if b[key] != value {
			return false
		}
	}
	return true
}
