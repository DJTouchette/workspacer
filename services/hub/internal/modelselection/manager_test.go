package modelselection

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"testing"
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
