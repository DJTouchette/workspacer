package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strconv"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// The brain answers most capability calls under the default catalog delegation
// (DELEGATE_CATALOG_TO_BRAIN) — it is THE provider for a bus caller, not a
// backup — and nothing scanned its params. capspec's params detector reads
// hubCapabilities.ts and only hubCapabilities.ts, so every dangerous field the
// Go handlers destructure was outside the reach of the drift machinery
// entirely: `bytesB64` (the base64 half of the PTY byte stream) is in this
// file's output and appears nowhere in the desktop provider at all.
//
// This is the Go half. It parses the dispatch switch in handlers.go, resolves
// each case to the params struct the handler actually unmarshals into, and
// requires every field whose json tag is in capspec's shared vocabulary to be
// classified — scoped by the bus, or excused per param with a kind and a
// reason.

// paramScan is the parsed brain: method name → the json tags its handler binds.
type paramScan struct {
	fset    *token.FileSet
	files   []*ast.File
	funcs   map[string]*ast.FuncDecl // registry method name → decl
	structs map[string]*ast.StructType
}

// parseBrain parses every non-test .go file in this package. Test files are
// excluded on purpose: a params struct in a _test.go is a fixture, not a
// surface a caller can reach.
func parseBrain(t *testing.T) *paramScan {
	t.Helper()
	fset := token.NewFileSet()
	entries, err := os.ReadDir(".")
	if err != nil {
		t.Fatalf("read package dir: %v", err)
	}
	ps := &paramScan{fset: fset, funcs: map[string]*ast.FuncDecl{}, structs: map[string]*ast.StructType{}}
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || filepath.Ext(name) != ".go" || strings.HasSuffix(name, "_test.go") {
			continue
		}
		f, err := parser.ParseFile(fset, name, nil, 0)
		if err != nil {
			t.Fatalf("parse %s: %v", name, err)
		}
		ps.files = append(ps.files, f)
	}
	if len(ps.files) == 0 {
		t.Fatal("parsed no brain source files — this scan is reading an empty package and would pass no matter what the handlers take")
	}
	for _, f := range ps.files {
		for _, d := range f.Decls {
			switch d := d.(type) {
			case *ast.FuncDecl:
				if d.Recv != nil {
					ps.funcs[d.Name.Name] = d
				}
			case *ast.GenDecl:
				for _, spec := range d.Specs {
					ts, ok := spec.(*ast.TypeSpec)
					if !ok {
						continue
					}
					if st, ok := ts.Type.(*ast.StructType); ok {
						ps.structs[ts.Name.Name] = st
					}
				}
			}
		}
	}
	return ps
}

// jsonTagsOf collects the json field names of a struct type expression,
// following named types (profileUpdate is reached through claude.profiles.update's
// `updates` wrapper, and its configDir/extraArgs are the fields that matter).
func (ps *paramScan) jsonTagsOf(expr ast.Expr, seen map[string]bool, out map[string]bool) {
	switch e := expr.(type) {
	case *ast.StarExpr:
		ps.jsonTagsOf(e.X, seen, out)
	case *ast.ArrayType:
		ps.jsonTagsOf(e.Elt, seen, out)
	case *ast.Ident:
		if seen[e.Name] {
			return
		}
		seen[e.Name] = true
		if st, ok := ps.structs[e.Name]; ok {
			ps.jsonTagsOf(st, seen, out)
		}
	case *ast.StructType:
		for _, f := range e.Fields.List {
			if f.Tag != nil {
				if raw, err := strconv.Unquote(f.Tag.Value); err == nil {
					tag := reflect.StructTag(raw).Get("json")
					if name := strings.SplitN(tag, ",", 2)[0]; name != "" && name != "-" {
						out[name] = true
					}
				}
			}
			ps.jsonTagsOf(f.Type, seen, out)
		}
	}
}

// paramsBoundIn returns the json tags of every value this node unmarshals the
// caller's params INTO. Anchoring on `unmarshal(raw, &p)` / json.Unmarshal is
// what keeps the scan precise: a handler also builds outbound structs
// (spawnReq{Argv: …}), and attributing those fields to the caller would flag
// params nobody can send.
func (ps *paramScan) paramsBoundIn(node ast.Node, depth int) map[string]bool {
	out := map[string]bool{}
	if node == nil || depth > 3 {
		return out
	}
	types := map[string]ast.Expr{} // local var name → declared type
	ast.Inspect(node, func(n ast.Node) bool {
		switch n := n.(type) {
		case *ast.DeclStmt:
			gd, ok := n.Decl.(*ast.GenDecl)
			if !ok {
				return true
			}
			for _, spec := range gd.Specs {
				vs, ok := spec.(*ast.ValueSpec)
				if !ok || vs.Type == nil {
					continue
				}
				for _, name := range vs.Names {
					types[name.Name] = vs.Type
				}
			}
		}
		return true
	})
	ast.Inspect(node, func(n ast.Node) bool {
		call, ok := n.(*ast.CallExpr)
		if !ok {
			return true
		}
		// The SOURCE has to be the caller's bytes too. providers.listModels
		// decodes claudemon's RESPONSE into `parsed` two statements later, and
		// counting that struct's `id` as a caller param is how a scan starts
		// demanding decisions for fields nobody can send — noise that gets a
		// detector switched off.
		if isUnmarshalCall(call) && len(call.Args) >= 2 && carriesCallerParams(call) {
			if id := addressedIdent(call.Args[len(call.Args)-1]); id != "" {
				if typ, ok := types[id]; ok {
					ps.jsonTagsOf(typ, map[string]bool{}, out)
				}
			}
		}
		// Follow r.someHandler(ctx, params): the dispatch switch is one line per
		// method, and the struct lives in the handler it calls. Only calls that
		// are HANDED the caller's bytes are followed — the fs.* cases also call
		// r.workspaceRoots(ctx), which unmarshals session snapshots, and
		// attributing that struct's `cwd` to fs.read would report a caller param
		// no caller can send.
		if sel, ok := call.Fun.(*ast.SelectorExpr); ok {
			if recv, ok := sel.X.(*ast.Ident); ok && recv.Name == "r" && carriesCallerParams(call) {
				if decl, ok := ps.funcs[sel.Sel.Name]; ok && decl.Body != nil {
					for tag := range ps.paramsBoundIn(decl.Body, depth+1) {
						out[tag] = true
					}
				}
			}
		}
		return true
	})
	return out
}

// carriesCallerParams reports whether a call is handed the raw caller payload —
// `params` in the dispatch switch, `raw` inside a handler. Following anything
// else walks into helpers that decode the daemon's OWN data.
func carriesCallerParams(call *ast.CallExpr) bool {
	for _, arg := range call.Args {
		if id, ok := arg.(*ast.Ident); ok && (id.Name == "params" || id.Name == "raw") {
			return true
		}
	}
	return false
}

func isUnmarshalCall(call *ast.CallExpr) bool {
	switch fn := call.Fun.(type) {
	case *ast.Ident:
		return fn.Name == "unmarshal"
	case *ast.SelectorExpr:
		return fn.Sel.Name == "Unmarshal"
	}
	return false
}

// addressedIdent pulls `p` out of `&p`.
func addressedIdent(e ast.Expr) string {
	u, ok := e.(*ast.UnaryExpr)
	if !ok || u.Op != token.AND {
		return ""
	}
	id, ok := u.X.(*ast.Ident)
	if !ok {
		return ""
	}
	return id.Name
}

// brainMethodParams maps each capability the dispatch switch answers to the
// json tags its handler binds from the caller's params.
func (ps *paramScan) brainMethodParams(t *testing.T) map[string]map[string]bool {
	t.Helper()
	handle, ok := ps.funcs["handle"]
	if !ok || handle.Body == nil {
		t.Fatal("no registry.handle found in the brain sources — the dispatch shape changed and this scan reads nothing")
	}
	out := map[string]map[string]bool{}
	ast.Inspect(handle.Body, func(n ast.Node) bool {
		cc, ok := n.(*ast.CaseClause)
		if !ok {
			return true
		}
		var methods []string
		for _, expr := range cc.List {
			lit, ok := expr.(*ast.BasicLit)
			if !ok || lit.Kind != token.STRING {
				continue
			}
			if name, err := strconv.Unquote(lit.Value); err == nil {
				methods = append(methods, name)
			}
		}
		if len(methods) == 0 {
			return true
		}
		block := &ast.BlockStmt{List: cc.Body}
		tags := ps.paramsBoundIn(block, 0)
		for _, m := range methods {
			if out[m] == nil {
				out[m] = map[string]bool{}
			}
			for tag := range tags {
				out[m][tag] = true
			}
		}
		return true
	})
	if len(out) == 0 {
		t.Fatal("parsed no capability cases out of registry.handle — the switch shape changed; this scan would pass over any surface at all")
	}
	return out
}

// TestBrainParamsAreClassified is TestCapabilitiesWithAPathParamAreClassified
// for the provider that actually answers. Every json field a brain handler
// binds from caller params, whose name is in capspec's shared vocabulary, must
// be classified per param.
//
// The two providers do NOT take identical params — sessions.terminalInput
// accepts `bytesB64` here and not in the desktop — so scanning one and calling
// it done was never sound.
func TestBrainParamsAreClassified(t *testing.T) {
	ps := parseBrain(t)
	byMethod := ps.brainMethodParams(t)

	methods := make([]string, 0, len(byMethod))
	for m := range byMethod {
		methods = append(methods, m)
	}
	sort.Strings(methods)

	flagged := 0
	for _, method := range methods {
		params := make([]string, 0, len(byMethod[method]))
		for p := range byMethod[method] {
			params = append(params, p)
		}
		sort.Strings(params)
		for _, param := range params {
			kind, dangerous := capspec.DangerousKind(param)
			if !dangerous {
				continue
			}
			flagged++
			if status, _ := capspec.ClassifyParam(method, param); status == capspec.ParamUnclassified {
				t.Errorf("the brain's %s handler binds %q (a %s in capspec's vocabulary) from caller params, and capspec classifies that PARAM nowhere — the decisions on record for %q are %v. Add a ParamDecision (kind + why), scope it in PathParam, or stop binding it.",
					method, param, kind, method, capspec.ParamDecisions(method))
			}
		}
	}
	// Canaries, one per parsing hop this scan depends on: a tag on an inline
	// struct in a handler reached through the switch (terminals.create.shell),
	// a tag on a handler that is only reachable by following r.x(params)
	// (sessions.load.filename), and a tag on a NAMED type nested inside the
	// params struct (claude.profiles.update.updates → profileUpdate.configDir).
	// Losing any one means the AST walk went blind to a whole class of handler
	// rather than that a capability was renamed.
	for _, c := range []struct{ method, param string }{
		{"terminals.create", "shell"},
		{"sessions.load", "filename"},
		{"claude.profiles.update", "configDir"},
		{"sessions.terminalInput", "bytesB64"},
	} {
		if !byMethod[c.method][c.param] {
			t.Errorf("the brain scan did not find %q on %s — the AST walk has stopped resolving a whole shape of handler, so every capability written that way is now unscanned", c.param, c.method)
		}
	}
	if flagged < 10 {
		t.Errorf("the brain scan classified only %d dangerous params; it used to see well over ten, so the parse has degraded into a test that passes because it looks at nothing", flagged)
	}
}
