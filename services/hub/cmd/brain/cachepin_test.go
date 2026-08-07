package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
)

// ---------------------------------------------------------------------------
// THE CACHE PIN, AS A CLASS.
//
// internal/extinput exists because cmd/go's test cache key drops every input
// outside the module root — computeTestInputsID bails at
// `search.InDir(name, a.Package.Root) == ""` — so a test that reads
// contracts/path-containment-cases.json, hubCapabilities.ts or the root
// Makefile with a plain os.ReadFile gets the right bytes and a cache key that
// has never heard of the file. The guard then reports a pass over a tree it did
// not read, forever, which is worse than no guard.
//
// That was fixed ONE SITE AT A TIME, and the fix therefore missed sites.
// Measured on the tree before this test existed: delete all 69 `expect: "deny"`
// cases from contracts/path-containment-cases.json and
// `go test ./internal/bus/` prints `ok (cached)` — the entire bus containment
// sweep, including the corpus floor of 107, unpinned — while cmd/brain and
// internal/capspec (already routed through extinput) fail. Touching
// contracts/README.md left EVERY package cached, and touching a Rust file left
// the "every fixture has two loaders" guard cached.
//
// A one-site fix cannot prevent the next site. This is the invariant instead:
// inside these packages, a path that ESCAPES the module root must not reach a
// raw os.ReadFile/os.ReadDir/os.Open/os.OpenFile. It has to go through
// sweepguard.ReadRepoFile or extinput, which read the same bytes through a path
// that still descends LEXICALLY from the module root — the only property
// search.InDir tests — so cmd/go hashes the file.
//
// The analysis is deliberately syntactic and deliberately narrow (it follows
// string literals with a ".." segment, sweepguard.Root/RepoPath results,
// filepath joins of either, and one level of helper-function return). It cannot
// see a path assembled through a slice or an interface, and it does not police
// parser.ParseFile, whose in-module inputs are pinned already. What it does
// catch is every shape this repo has actually shipped — and unlike a comment,
// it fails.
// ---------------------------------------------------------------------------

// cachePinSweptDirs are the Go test packages that read repo files from ABOVE
// this module. internal/sweepguard and internal/extinput are excluded on
// purpose: they are the implementation of the pin, and their own tests read
// through raw syscalls to prove the mechanism.
var cachePinSweptDirs = []string{
	filepath.Join("services", "hub", "cmd", "brain"),
	filepath.Join("services", "hub", "internal", "bus"),
	filepath.Join("services", "hub", "internal", "capspec"),
}

// rawReaders are the os calls that open a file without cmd/go being able to
// re-check it when the path escapes the module root.
var rawReaders = map[string]bool{
	"ReadFile": true,
	"ReadDir":  true,
	"Open":     true,
	"OpenFile": true,
}

// cachePinEscapingNamesFloor is how many escaping consts/vars/helpers the
// analysis finds across the swept packages today. Without it, an analyzer that
// silently stopped resolving anything would report zero findings and pass —
// the same "guarded nothing, printed ok" shape one level up. The synthetic
// self-test below is the other half.
const cachePinEscapingNamesFloor = 4

type cachePinFinding struct {
	file string
	line int
	call string
	arg  string
}

// TestNoCrossRepoReadBypassesTheTestCachePin is the guard.
func TestNoCrossRepoReadBypassesTheTestCachePin(t *testing.T) {
	// Half one: the analysis must be able to SEE the defect. A checker that
	// resolves nothing reports a clean tree, and there is no way to tell that
	// apart from a clean tree by reading the pass.
	t.Run("the analysis is falsifiable", func(t *testing.T) {
		guilty := `package p

import (
	"os"
	"path/filepath"

	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

const fixtureRel = "../../../../contracts/path-containment-cases.json"

func fixtureDir() string { return filepath.Join("..", "..", "..", "..", "contracts") }

func literalRead() { _, _ = os.ReadFile(fixtureRel) }

func helperRead() { _, _ = os.ReadDir(fixtureDir()) }

func joinedRead() { _, _ = os.ReadFile(filepath.Join(fixtureDir(), "README.md")) }

func rootDerivedRead() {
	root, _ := sweepguard.Root()
	dir := filepath.Join(root, "contracts")
	_, _ = os.ReadDir(dir)
}
`
		findings, _ := runCachePinAnalysis(t, "guilty.go", guilty)
		if len(findings) != 4 {
			t.Fatalf("the analysis found %d of 4 planted bypasses (%+v) — a bare \"../../../..\" literal, a helper that returns one, a filepath.Join of that helper, and a sweepguard.Root-derived directory. Every one of those shapes has shipped in this repo, so an analysis that misses any of them makes the clean verdict below meaningless", len(findings), findings)
		}

		innocent := `package p

import (
	"os"

	"github.com/djtouchette/workspacer-hub/internal/extinput"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

func pinnedRead(t interface{}) {
	_, _ = sweepguard.ReadRepoFile("contracts", "path-containment-cases.json")
	dir, _ := sweepguard.RepoPath("contracts")
	_, _ = extinput.ReadDir(dir)
	_, _ = extinput.ReadFile("../../../../contracts/path-containment-cases.json")
	// A sandbox path is not a repo path, and must never be flagged.
	_, _ = os.ReadFile("/tmp/sandbox/out.txt")
}
`
		clean, _ := runCachePinAnalysis(t, "innocent.go", innocent)
		if len(clean) != 0 {
			t.Fatalf("the analysis flagged %d compliant reads (%+v) — a guard that fires on the correct form is one the next person turns off", len(clean), clean)
		}
	})

	// Half two: the real tree.
	totalNames := 0
	for _, dir := range cachePinSweptDirs {
		files := goTestFiles(t, dir)
		fset := token.NewFileSet()
		var parsed []*ast.File
		for _, file := range files {
			f, err := parser.ParseFile(fset, file, nil, 0)
			if err != nil {
				t.Fatalf("parse %s: %v", file, err)
			}
			parsed = append(parsed, f)
		}
		findings, names := analyzeCachePin(fset, parsed)
		totalNames += names
		for _, f := range findings {
			t.Errorf("%s:%d: os.%s(%s) reads a path that ESCAPES the Go module root. cmd/go drops such inputs from the test cache key (search.InDir), so this read is invisible to `go test` and the guard around it reports `ok (cached)` over bytes it never looked at — which is how the bus containment corpus could lose all 69 of its deny cases and stay green. Use sweepguard.ReadRepoFile / mustReadRepoFile for bytes, extinput.ReadDir / walkPinned for a listing.",
				relTo(t, f.file), f.line, f.call, f.arg)
		}
	}
	t.Logf("resolved %d module-escaping paths across %v with no raw read of any of them", totalNames, cachePinSweptDirs)
	if totalNames < cachePinEscapingNamesFloor {
		t.Fatalf("the analysis resolved only %d escaping paths across %v (floor %d) — it is reading the wrong packages, or it has stopped resolving, and either way its clean verdict is worthless",
			totalNames, cachePinSweptDirs, cachePinEscapingNamesFloor)
	}
}

// runCachePinAnalysis parses one in-memory source and analyses it.
func runCachePinAnalysis(t *testing.T, name, src string) ([]cachePinFinding, int) {
	t.Helper()
	fset := token.NewFileSet()
	f, err := parser.ParseFile(fset, name, src, 0)
	if err != nil {
		t.Fatalf("parse %s: %v", name, err)
	}
	return analyzeCachePin(fset, []*ast.File{f})
}

// analyzeCachePin returns every raw os read of a module-escaping path, and how
// many escaping names it managed to resolve (the number that makes a "no
// findings" answer worth anything).
func analyzeCachePin(fset *token.FileSet, files []*ast.File) ([]cachePinFinding, int) {
	pkgEscaping := map[string]bool{}
	escapingFuncs := map[string]bool{}

	var isEscaping func(e ast.Expr, local map[string]bool) bool
	isEscaping = func(e ast.Expr, local map[string]bool) bool {
		switch x := e.(type) {
		case *ast.ParenExpr:
			return isEscaping(x.X, local)
		case *ast.BasicLit:
			if x.Kind != token.STRING {
				return false
			}
			s, err := strconv.Unquote(x.Value)
			if err != nil {
				return false
			}
			for _, seg := range strings.Split(filepath.ToSlash(s), "/") {
				if seg == ".." {
					return true
				}
			}
			return false
		case *ast.Ident:
			return local[x.Name] || pkgEscaping[x.Name]
		case *ast.BinaryExpr:
			return isEscaping(x.X, local) || isEscaping(x.Y, local)
		case *ast.CallExpr:
			switch fn := x.Fun.(type) {
			case *ast.Ident:
				return escapingFuncs[fn.Name]
			case *ast.SelectorExpr:
				pkg, ok := fn.X.(*ast.Ident)
				if !ok {
					return false
				}
				// The monorepo root, and anything built from it, is out of
				// module by construction — that is the whole point of Root().
				if pkg.Name == "sweepguard" && (fn.Sel.Name == "Root" || fn.Sel.Name == "RepoPath") {
					return true
				}
				// path/filepath only ever propagates its arguments.
				if pkg.Name == "filepath" {
					for _, a := range x.Args {
						if isEscaping(a, local) {
							return true
						}
					}
				}
			}
		}
		return false
	}

	// localEscaping is the per-function set, built in source order.
	localEscaping := func(fn *ast.FuncDecl) map[string]bool {
		local := map[string]bool{}
		mark := func(lhs, rhs []ast.Expr) {
			if len(rhs) == 0 {
				return
			}
			if len(lhs) == len(rhs) {
				for i := range lhs {
					if id, ok := lhs[i].(*ast.Ident); ok && isEscaping(rhs[i], local) {
						local[id.Name] = true
					}
				}
				return
			}
			// `a, b := f()`: one call, several results. A helper that returns a
			// repo path alongside anything else taints both, which is what
			// contractsDir(t) (repoRoot, contractsDir) does.
			if isEscaping(rhs[0], local) {
				for _, l := range lhs {
					if id, ok := l.(*ast.Ident); ok {
						local[id.Name] = true
					}
				}
			}
		}
		ast.Inspect(fn, func(n ast.Node) bool {
			switch s := n.(type) {
			case *ast.AssignStmt:
				mark(s.Lhs, s.Rhs)
			case *ast.ValueSpec:
				names := make([]ast.Expr, 0, len(s.Names))
				for _, id := range s.Names {
					names = append(names, id)
				}
				mark(names, s.Values)
			}
			return true
		})
		return local
	}

	// Fixpoint over package-level names and helper returns. Four rounds is far
	// more than this repo's one-hop helpers need, and it terminates regardless.
	for round := 0; round < 4; round++ {
		changed := false
		for _, f := range files {
			for _, d := range f.Decls {
				switch decl := d.(type) {
				case *ast.GenDecl:
					if decl.Tok != token.CONST && decl.Tok != token.VAR {
						continue
					}
					for _, spec := range decl.Specs {
						vs, ok := spec.(*ast.ValueSpec)
						if !ok {
							continue
						}
						for i, name := range vs.Names {
							if i >= len(vs.Values) || pkgEscaping[name.Name] {
								continue
							}
							if isEscaping(vs.Values[i], nil) {
								pkgEscaping[name.Name] = true
								changed = true
							}
						}
					}
				case *ast.FuncDecl:
					if decl.Body == nil || escapingFuncs[decl.Name.Name] {
						continue
					}
					local := localEscaping(decl)
					escapes := false
					ast.Inspect(decl.Body, func(n ast.Node) bool {
						ret, ok := n.(*ast.ReturnStmt)
						if !ok {
							return true
						}
						for _, r := range ret.Results {
							if isEscaping(r, local) {
								escapes = true
							}
						}
						return true
					})
					if escapes {
						escapingFuncs[decl.Name.Name] = true
						changed = true
					}
				}
			}
		}
		if !changed {
			break
		}
	}

	var findings []cachePinFinding
	for _, f := range files {
		for _, d := range f.Decls {
			fn, ok := d.(*ast.FuncDecl)
			if !ok || fn.Body == nil {
				continue
			}
			local := localEscaping(fn)
			ast.Inspect(fn.Body, func(n ast.Node) bool {
				call, ok := n.(*ast.CallExpr)
				if !ok {
					return true
				}
				sel, ok := call.Fun.(*ast.SelectorExpr)
				if !ok {
					return true
				}
				pkg, ok := sel.X.(*ast.Ident)
				if !ok || pkg.Name != "os" || !rawReaders[sel.Sel.Name] {
					return true
				}
				for _, a := range call.Args {
					if !isEscaping(a, local) {
						continue
					}
					pos := fset.Position(call.Pos())
					findings = append(findings, cachePinFinding{
						file: pos.Filename,
						line: pos.Line,
						call: sel.Sel.Name,
						arg:  exprText(a),
					})
					break
				}
				return true
			})
		}
	}
	return findings, len(pkgEscaping) + len(escapingFuncs)
}

// exprText renders an expression well enough to name it in a failure.
func exprText(e ast.Expr) string {
	switch x := e.(type) {
	case *ast.Ident:
		return x.Name
	case *ast.BasicLit:
		return x.Value
	case *ast.SelectorExpr:
		return exprText(x.X) + "." + x.Sel.Name
	case *ast.CallExpr:
		args := make([]string, 0, len(x.Args))
		for _, a := range x.Args {
			args = append(args, exprText(a))
		}
		return exprText(x.Fun) + "(" + strings.Join(args, ", ") + ")"
	case *ast.BinaryExpr:
		return exprText(x.X) + " " + x.Op.String() + " " + exprText(x.Y)
	}
	return "<expr>"
}
