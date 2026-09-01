package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"strconv"
	"strings"
	"testing"
)

var windowsOrdinalContainmentContract = []struct {
	name, a, b string
	want       bool
}{
	{"ASCII", `C:\Work\Manager`, `c:\work\manager`, true},
	{"Cyrillic", `C:\Работа\ПРОЕКТ`, `c:\работа\проект`, true},
	{"umlaut", `C:\Work\Ärger`, `c:\work\ärGER`, true},
	{"Kelvin sign is not K", `C:\Work\Kelvin`, `c:\work\kelvin`, false},
	{"different drive", `D:\Work\Manager`, `C:\Work\Manager`, false},
}

// This host-independent contract makes the Windows policy reviewable on Linux:
// it kills both an ASCII-only comparison and generic Unicode case folding. The
// Windows-only integration test below covers the Fleet Manager's drive-letter,
// slash, and directory-case spellings through all three brief calls, plus its
// outside-workspace denial.
func TestWindowsContainmentContractRejectsUnsafeComparators(t *testing.T) {
	asciiMutationKilled := false
	equalFoldMutationKilled := false
	for _, tc := range windowsOrdinalContainmentContract {
		if windowsASCIIEqual(tc.a, tc.b) != tc.want {
			asciiMutationKilled = true
		}
		if strings.EqualFold(tc.a, tc.b) != tc.want {
			equalFoldMutationKilled = true
		}
	}
	if !asciiMutationKilled {
		t.Error("Windows contract does not kill an ASCII-only path comparison")
	}
	if !equalFoldMutationKilled {
		t.Error("Windows contract does not kill strings.EqualFold; it treats Kelvin sign as K")
	}
}

// Linux CI cannot invoke kernel32, so pin the production boundary structurally
// as well as compiling it in the Windows cross-build. This prevents a future
// refactor from quietly replacing the native ordinal comparison with a generic
// Unicode fold that changes filesystem authorization.
func TestWindowsContainmentUsesCompareStringOrdinalIgnoreCase(t *testing.T) {
	f, err := parser.ParseFile(token.NewFileSet(), "pathmatch_windows.go", nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	foundProc := false
	foundIgnoreCaseCall := false
	ast.Inspect(f, func(node ast.Node) bool {
		call, ok := node.(*ast.CallExpr)
		if !ok {
			return true
		}
		if sel, ok := call.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == "NewProc" && len(call.Args) == 1 {
			if lit, ok := call.Args[0].(*ast.BasicLit); ok {
				if name, unquoteErr := strconv.Unquote(lit.Value); unquoteErr == nil && name == "CompareStringOrdinal" {
					foundProc = true
				}
			}
		}
		if sel, ok := call.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == "Call" && len(call.Args) == 5 {
			receiver, receiverOK := sel.X.(*ast.Ident)
			ignoreCase, ignoreCaseOK := call.Args[4].(*ast.BasicLit)
			if receiverOK && receiver.Name == "procCompareStringOrdinal" && ignoreCaseOK && ignoreCase.Value == "1" {
				foundIgnoreCaseCall = true
			}
		}
		return true
	})
	if !foundProc {
		t.Fatal("pathmatch_windows.go does not resolve kernel32 CompareStringOrdinal")
	}
	if !foundIgnoreCaseCall {
		t.Fatal("CompareStringOrdinal is not called with bIgnoreCase=TRUE")
	}
}

func windowsASCIIEqual(a, b string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := 0; i < len(a); i++ {
		ac, bc := a[i], b[i]
		if ac >= 'A' && ac <= 'Z' {
			ac += 'a' - 'A'
		}
		if bc >= 'A' && bc <= 'Z' {
			bc += 'a' - 'A'
		}
		if ac != bc {
			return false
		}
	}
	return true
}

// windowsContainmentContract is the containment DECISION the Windows Fleet
// Manager path depends on, spelled as Windows paths and asserted twice: by the
// mutation proof below, which runs anywhere, and by
// TestWindowsPathContainmentMatchesTheOrdinalContract, which runs the shipping
// containsPath against it on the one host that can call kernel32.
//
// Every vector is pure ASCII on purpose. The Unicode discrimination (Kelvin
// sign, umlauts, Cyrillic) is the COMPARATOR's job and lives in
// windowsOrdinalContainmentContract above; this table is about what containment
// does with the comparator's answer, which is where a widening hides.
var windowsContainmentContract = []struct {
	name, root, target string
	want               bool
}{
	{"the manager root itself, spelled with a lower-case drive", `C:\Work\FleetManager`, `c:\work\fleetmanager`, true},
	{"a project under the manager root, case-swapped", `C:\Work\FleetManager`, `c:\WORK\fleetMANAGER\Projects\Client`, true},
	{"a sibling whose path is the root plus more letters", `C:\Work\FleetManager`, `C:\Work\FleetManagerOther\loot.txt`, false},
	{"the same widening spelled in another case", `C:\Work\FleetManager`, `c:\work\fleetmanagerother\loot.txt`, false},
	{"a directory outside the root entirely", `C:\Work\FleetManager`, `C:\Work\Outside\loot.txt`, false},
	{"a volume-prefix root contains everything below it", `C:\`, `C:\Work\loot.txt`, true},
	{"another drive is not inside it", `C:\Work`, `D:\Work\loot.txt`, false},
}

// windowsContainsPath is containsPath's shape with its two platform primitives
// injected and its separator fixed to the Windows one, so a Linux runner can
// execute the Windows containment decision and a MUTATION of it. It is a copy
// of six lines rather than a call because the point is to run variants the
// shipping code must not have; the shipping code is asserted against the same
// vectors on Windows.
func windowsContainsPath(root, target string, equal func(a, b string) bool, hasPrefix func(path, prefix string) bool, separatorRequired bool) bool {
	if root == "" {
		return false
	}
	if equal(target, root) {
		return true
	}
	if strings.HasSuffix(root, `\`) {
		return hasPrefix(target, root)
	}
	if !separatorRequired {
		return hasPrefix(target, root)
	}
	return hasPrefix(target, root+`\`)
}

// TestWindowsContainmentContractKillsBothContainmentMutations is the mutation
// proof for windowsContainmentContract, and it insists on the two DIRECTIONS
// separately, because one vector set can easily catch only one of them:
//
//   - BYTE-EXACT containment (the pre-fix comparison, and the state anything
//     reverting pathmatch_windows.go to pathmatch_other.go's bodies lands in)
//     must be caught by a vector the contract expects to ALLOW. That is a false
//     denial — the Fleet Manager refused its own workspace — and a table with
//     only deny vectors would not notice, since byte-exactness denies more.
//   - DROPPING THE SEPARATOR containsPath appends to a non-volume root must be
//     caught by a vector the contract expects to DENY. That is the widening
//     folding the comparison makes reachable without a "..", and a table with
//     only allow vectors would not notice either.
//
// Naming the killing vectors rather than counting them is deliberate: a
// mutation "killed" by a vector of the wrong polarity proves nothing.
func TestWindowsContainmentContractKillsBothContainmentMutations(t *testing.T) {
	var falseDenials, widenings []string
	for _, tc := range windowsContainmentContract {
		if got := windowsContainsPath(tc.root, tc.target, windowsASCIIEqual, windowsASCIIHasPrefix, true); got != tc.want {
			t.Errorf("the contract disagrees with itself: containment(%q, %q) = %v, want %v — %s", tc.root, tc.target, got, tc.want, tc.name)
		}
		byteExact := windowsContainsPath(tc.root, tc.target, func(a, b string) bool { return a == b }, strings.HasPrefix, true)
		if byteExact != tc.want && tc.want {
			falseDenials = append(falseDenials, tc.name)
		}
		widened := windowsContainsPath(tc.root, tc.target, windowsASCIIEqual, windowsASCIIHasPrefix, false)
		if widened != tc.want && !tc.want {
			widenings = append(widenings, tc.name)
		}
	}
	if len(falseDenials) == 0 {
		t.Error("no vector catches byte-exact containment as a FALSE DENIAL — the contract would pass with the Windows fold reverted")
	}
	if len(widenings) == 0 {
		t.Error("no vector catches a dropped root separator as a WIDENING — the contract would pass with an unsafe ordinal prefix test")
	}
	t.Logf("byte-exact containment killed by: %v; dropped separator killed by: %v", falseDenials, widenings)
}

func windowsASCIIHasPrefix(path, prefix string) bool {
	return len(path) >= len(prefix) && windowsASCIIEqual(path[:len(prefix)], prefix)
}
