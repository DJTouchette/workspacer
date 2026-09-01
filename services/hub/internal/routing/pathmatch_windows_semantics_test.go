package routing

import (
	"go/ast"
	"go/parser"
	"go/token"
	"strconv"
	"strings"
	"testing"
)

var windowsOrdinalRoutingContract = []struct {
	name string
	a    string
	b    string
	want bool
}{
	{name: "ASCII", a: `C:\Work\Client`, b: `c:\work\client`, want: true},
	{name: "Cyrillic", a: `C:\Работа\ПРОЕКТ`, b: `c:\работа\проект`, want: true},
	{name: "umlaut", a: `C:\Work\Ärger`, b: `c:\work\ärGER`, want: true},
	{name: "Kelvin sign is not K", a: `C:\Work\Kelvin`, b: `c:\work\kelvin`, want: false},
	{name: "different drive", a: `D:\work\client`, b: `C:\work\client`, want: false},
	{name: "different UNC share", a: `\\server\other\work`, b: `\\server\share\work`, want: false},
}

// This contract runs on every host and kills both unsafe implementations this
// code has had. The Windows-only test below drives the same rows through the
// native implementation; keeping the mutants here means Linux CI still proves
// why neither a bytewise ASCII fold nor strings.EqualFold is an acceptable
// replacement.
func TestWindowsOrdinalRoutingContractKillsUnsafeAlternatives(t *testing.T) {
	asciiMutationKilled := false
	equalFoldMutationKilled := false
	for _, tc := range windowsOrdinalRoutingContract {
		if windowsASCIIOnlyEqual(tc.a, tc.b) != tc.want {
			asciiMutationKilled = true
		}
		if strings.EqualFold(tc.a, tc.b) != tc.want {
			equalFoldMutationKilled = true
		}
	}
	if !asciiMutationKilled {
		t.Error("contract does not kill the ASCII-only mutation; a Unicode case variant could miss its ceiling")
	}
	if !equalFoldMutationKilled {
		t.Error("contract does not kill the strings.EqualFold mutation; Kelvin sign could falsely match K")
	}
}

// Host-independent structural coverage pins the OS boundary itself. Without
// this guard, a Linux-only run could keep the table above green while production
// quietly stopped calling the Windows ordinal primitive.
func TestWindowsRoutingComparisonUsesCompareStringOrdinalIgnoreCase(t *testing.T) {
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
				name, unquoteErr := strconv.Unquote(lit.Value)
				if unquoteErr == nil && name == "CompareStringOrdinal" {
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
		t.Fatal("CompareStringOrdinal is not called with bIgnoreCase=TRUE (numeric 1)")
	}
}

func windowsASCIIOnlyEqual(a, b string) bool {
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
