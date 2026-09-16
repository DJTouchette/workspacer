package bus

// The production bus grants ambient access to every authenticated identity.
// This file covers the remaining active primitives (canonicalization and
// unambiguous parameter decoding) plus a small compatibility check for the old
// anonymous capGrant harness. It intentionally does not claim that manifest
// roots or secret-path filters authorize agents or enabled plugins.

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

func TestWithinUsesComponentBoundary(t *testing.T) {
	sep := string(os.PathSeparator)
	root := sep + filepath.Join("srv", "repo")
	if !within(root, filepath.Join(root, "file")) {
		t.Fatal("selected-object child was not contained")
	}
	if within(root, root+"s"+sep+"file") {
		t.Fatal("sibling prefix was treated as contained")
	}
}

func TestLegacyAnonymousHarnessStillFailsClosed(t *testing.T) {
	root := t.TempDir()
	canonicalRoot, ok := canonicalizeRoot(root)
	if !ok {
		t.Fatal("could not canonicalize compatibility root")
	}
	if allowed, err := pathWithinRoots([]string{canonicalRoot}, filepath.Join(root, "file")); err != nil || !allowed {
		t.Fatalf("inside compatibility path: allowed=%v err=%v", allowed, err)
	}
	if allowed, _ := pathWithinRoots([]string{canonicalRoot}, filepath.Join(t.TempDir(), "file")); allowed {
		t.Fatal("legacy anonymous harness admitted a path outside its compatibility root")
	}
}

type paramShapeCase struct {
	Name         string          `json:"name"`
	Field        string          `json:"field"`
	Params       json.RawMessage `json:"params"`
	ParamsAbsent bool            `json:"paramsAbsent"`
	Expect       string          `json:"expect"`
}

func TestParamShapeContractCases(t *testing.T) {
	raw, err := sweepguard.ReadRepoFile("contracts", "path-containment-cases.json")
	if err != nil {
		t.Fatal(err)
	}
	var fx struct {
		ParamShapes []paramShapeCase `json:"paramShapes"`
	}
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatal(err)
	}
	if len(fx.ParamShapes) < 17 {
		t.Fatalf("param shape corpus has %d cases, want at least 17", len(fx.ParamShapes))
	}
	for _, testCase := range fx.ParamShapes {
		t.Run(testCase.Name, func(t *testing.T) {
			params := testCase.Params
			if testCase.ParamsAbsent {
				params = nil
			}
			_, ok := paramString(params, testCase.Field)
			if want := testCase.Expect == "accept"; ok != want {
				t.Fatalf("paramString accepted=%v, want %v", ok, want)
			}
		})
	}
}

func TestMaxLinkHopsMatchesTheFixture(t *testing.T) {
	raw, err := sweepguard.ReadRepoFile("contracts", "path-containment-cases.json")
	if err != nil {
		t.Fatal(err)
	}
	var fx struct {
		MaxLinkHops int `json:"maxLinkHops"`
	}
	if err := json.Unmarshal(raw, &fx); err != nil {
		t.Fatal(err)
	}
	if fx.MaxLinkHops != maxLinkHops {
		t.Fatalf("maxLinkHops = %d, contract = %d", maxLinkHops, fx.MaxLinkHops)
	}
}
