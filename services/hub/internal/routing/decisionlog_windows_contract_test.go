package routing

import (
	"go/ast"
	"go/parser"
	"go/token"
	"testing"
)

// This host-independent guard makes the creation-time privacy contract
// mutation-testable where Windows binaries can only be cross-compiled. The
// Windows runtime tests remain the oracle for the ACL that CreateFile installs;
// this test pins the atomic part: the descriptor must travel in CreateFile's
// SECURITY_ATTRIBUTES argument rather than being installed after open.
func TestWindowsDecisionLogCreateFileGetsSecurityAttributes(t *testing.T) {
	f, err := parser.ParseFile(token.NewFileSet(), "decisionlog_private_windows.go", nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	foundCreate := false
	ast.Inspect(f, func(node ast.Node) bool {
		call, ok := node.(*ast.CallExpr)
		if !ok || len(call.Args) < 4 {
			return true
		}
		sel, ok := call.Fun.(*ast.SelectorExpr)
		if !ok || sel.Sel.Name != "CreateFile" {
			return true
		}
		foundCreate = true
		addr, ok := call.Args[3].(*ast.UnaryExpr)
		if !ok || addr.Op != token.AND {
			t.Errorf("CreateFile SECURITY_ATTRIBUTES argument = %T, want an inline protected descriptor (a nil mutation reopens the inherited-ACL window)", call.Args[3])
			return false
		}
		lit, ok := addr.X.(*ast.CompositeLit)
		if !ok {
			t.Errorf("CreateFile SECURITY_ATTRIBUTES pointer targets %T, want windows.SecurityAttributes", addr.X)
			return false
		}
		attrType, ok := lit.Type.(*ast.SelectorExpr)
		if !ok || attrType.Sel.Name != "SecurityAttributes" {
			t.Errorf("CreateFile SECURITY_ATTRIBUTES literal type = %T, want windows.SecurityAttributes", lit.Type)
			return false
		}
		foundDescriptor := false
		for _, elt := range lit.Elts {
			kv, ok := elt.(*ast.KeyValueExpr)
			if !ok {
				continue
			}
			key, keyOK := kv.Key.(*ast.Ident)
			value, valueOK := kv.Value.(*ast.Ident)
			if keyOK && valueOK && key.Name == "SecurityDescriptor" && value.Name == "securityDescriptor" {
				foundDescriptor = true
			}
		}
		if !foundDescriptor {
			t.Error("Windows SecurityAttributes does not carry the private security descriptor")
		}
		return false
	})
	if !foundCreate {
		t.Fatal("no windows.CreateFile call found in decisionlog_private_windows.go")
	}
}
