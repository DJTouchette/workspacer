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
