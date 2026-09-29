// Source-only reference oracle for the original Go brain binding inventory.
// Extraction preserves the original AST walk, including its documented depth bound.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strconv"
	"strings"
)

var brainDir string

type scanError struct{}

func (*scanError) Helper()                           {}
func (*scanError) Fatal(args ...any)                 { panic(fmt.Sprint(args...)) }
func (*scanError) Fatalf(format string, args ...any) { panic(fmt.Sprintf(format, args...)) }

type paramScan struct {
	fset    *token.FileSet
	files   []*ast.File
	funcs   map[string]*ast.FuncDecl // registry method name → decl
	structs map[string]*ast.StructType
}

// parseBrain parses every non-test .go file in this package. Test files are
// excluded on purpose: a params struct in a _test.go is a fixture, not a
// surface a caller can reach.
func parseBrain(t *scanError) *paramScan {
	t.Helper()
	fset := token.NewFileSet()
	entries, err := os.ReadDir(brainDir)
	if err != nil {
		t.Fatalf("read package dir: %v", err)
	}
	ps := &paramScan{fset: fset, funcs: map[string]*ast.FuncDecl{}, structs: map[string]*ast.StructType{}}
	for _, e := range entries {
		name := e.Name()
		if e.IsDir() || filepath.Ext(name) != ".go" || strings.HasSuffix(name, "_test.go") {
			continue
		}
		f, err := parser.ParseFile(fset, filepath.Join(brainDir, name), nil, 0)
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

// jsonTagsOf collects the caller keys a struct type BINDS, following named types
// (profileUpdate is reached through claude.profiles.update's `updates` wrapper,
// and its configDir/extraArgs are the fields that matter).
//
// A field with no json tag is collected under its Go NAME, and every name is
// matched case-insensitively downstream, because that is what encoding/json
// does: it prefers an exact tag/name match and then falls back to a
// case-INSENSITIVE one, so `Env string` with no tag, and `Env string
// \`json:"Env"\`, both receive the caller's {"env": …}. Collecting only exact
// tag spellings meant either spelling read as "not in the vocabulary" and the
// param was classified by nobody — a rename away from a silent disarm.
func (ps *paramScan) jsonTagsOf(expr ast.Expr, seen map[string]bool, out map[string]bool) {
	switch e := expr.(type) {
	case *ast.StarExpr:
		ps.jsonTagsOf(e.X, seen, out)
	case *ast.ArrayType:
		ps.jsonTagsOf(e.Elt, seen, out)
	case *ast.MapType:
		ps.jsonTagsOf(e.Value, seen, out)
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
			tagged, skip := "", false
			if f.Tag != nil {
				if raw, err := strconv.Unquote(f.Tag.Value); err == nil {
					tag := reflect.StructTag(raw).Get("json")
					switch name := strings.SplitN(tag, ",", 2)[0]; name {
					case "-":
						skip = true
					case "":
					default:
						tagged = name
					}
				}
			}
			switch {
			case skip:
			case tagged != "":
				out[tagged] = true
			default:
				// No usable tag: encoding/json binds by field name.
				for _, n := range f.Names {
					if n.IsExported() {
						out[n.Name] = true
					}
				}
			}
			ps.jsonTagsOf(f.Type, seen, out)
		}
	}
}

// boundParams is what a handler binds from the caller's payload: the keys it
// names, and whether it swallowed the WHOLE payload into a map without ever
// naming a key.
type boundParams struct {
	keys map[string]bool
	// opaque is the config.save shape: `var partial map[string]any` unmarshalled
	// from the caller's bytes and handed on whole. No key is ever spelled, so
	// there is nothing for a name-based scan to flag — which is precisely why
	// config.save's agents.binaries (argv[0] of every spawned agent) was
	// invisible to this machinery. A method that binds one owes its decisions to
	// a different check; see TestBrainOpaquePayloadHandlersAreClassified.
	opaque bool
}

func newBoundParams() boundParams { return boundParams{keys: map[string]bool{}} }

func (b *boundParams) merge(o boundParams) {
	for k := range o.keys {
		b.keys[k] = true
	}
	b.opaque = b.opaque || o.opaque
}

// paramsBoundIn returns the caller keys every value this node unmarshals the
// caller's params INTO binds. Anchoring on `unmarshal(raw, &p)` / json.Unmarshal
// is what keeps the scan precise: a handler also builds outbound structs
// (spawnReq{Argv: …}), and attributing those fields to the caller would flag
// params nobody can send.
//
// Two target shapes, not one. A struct target gives its json tags (jsonTagsOf).
// A map[string]any target gives the string literals the handler INDEXES it with
// — `input["name"]`, `p["agents"]` — which is the only shape layouts.save and
// sessions.save are written in. The scan used to understand structs only, so
// those two, plus config.save, produced an EMPTY key list and passed by looking
// at nothing: layouts.save's `name` and `id` both reach layoutFilePath.
func (ps *paramScan) paramsBoundIn(node ast.Node, depth int) boundParams {
	out := newBoundParams()
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
					if _, isMap := typ.(*ast.MapType); isMap {
						keys := literalIndexKeys(node, id)
						for k := range keys {
							out.keys[k] = true
						}
						if len(keys) == 0 {
							out.opaque = true
						}
					} else {
						ps.jsonTagsOf(typ, map[string]bool{}, out.keys)
					}
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
					out.merge(ps.paramsBoundIn(decl.Body, depth+1))
				}
			}
		}
		return true
	})
	return out
}

// literalIndexKeys collects the string literals a map-typed local is indexed
// with anywhere in node: `input["name"]`, `str(p["id"])`. Those literals ARE the
// caller keys the handler reads, and they are all a map-shaped handler ever says
// about its params.
func literalIndexKeys(node ast.Node, mapVar string) map[string]bool {
	out := map[string]bool{}
	ast.Inspect(node, func(n ast.Node) bool {
		ix, ok := n.(*ast.IndexExpr)
		if !ok {
			return true
		}
		id, ok := ix.X.(*ast.Ident)
		if !ok || id.Name != mapVar {
			return true
		}
		lit, ok := ix.Index.(*ast.BasicLit)
		if !ok || lit.Kind != token.STRING {
			return true
		}
		if key, err := strconv.Unquote(lit.Value); err == nil && key != "" {
			out[key] = true
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
// caller keys its handler binds.
func (ps *paramScan) brainMethodParams(t *scanError) map[string]boundParams {
	t.Helper()
	handle, ok := ps.funcs["handle"]
	if !ok || handle.Body == nil {
		t.Fatal("no registry.handle found in the brain sources — the dispatch shape changed and this scan reads nothing")
	}
	out := map[string]boundParams{}
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
		bound := ps.paramsBoundIn(block, 0)
		for _, m := range methods {
			cur, ok := out[m]
			if !ok {
				cur = newBoundParams()
			}
			cur.merge(bound)
			out[m] = cur
		}
		return true
	})
	if len(out) == 0 {
		t.Fatal("parsed no capability cases out of registry.handle — the switch shape changed; this scan would pass over any surface at all")
	}
	return out
}

type binding struct {
	Fields    []string `json:"fields"`
	Dangerous []string `json:"dangerous"`
	Opaque    bool     `json:"opaque"`
}
type reference struct {
	ScannerSHA256     string             `json:"scannerSha256"`
	Sources           map[string]string  `json:"sources"`
	VocabularySHA256  string             `json:"vocabularySha256"`
	DangerousBindings int                `json:"dangerousBindings"`
	Methods           map[string]binding `json:"methods"`
}

func digest(b []byte) string { h := sha256.Sum256(b); return hex.EncodeToString(h[:]) }
func read(path string) []byte {
	b, e := os.ReadFile(path)
	if e != nil {
		panic(e)
	}
	return b
}
func main() {
	root := flag.String("root", ".", "repository root")
	check := flag.String("check", "", "compare against a captured JSON file")
	flag.Parse()
	scanner := read(filepath.Join(*root, "services/hub/cmd/brain/capspec_params_test.go"))
	if digest(scanner) != "17895a08226980104703cb21694a028c406ad6415f9e3902b157189b266f03f5" {
		panic("original scanner changed: review and regenerate the AST extraction before capturing")
	}
	brainDir = filepath.Join(*root, "services/hub/cmd/brain")
	vocabBytes := read(filepath.Join(*root, "apps/desktop/tests/fixtures/capability-parameter-vocabulary.json"))
	var vocabulary struct {
		Params map[string]string `json:"params"`
	}
	if e := json.Unmarshal(vocabBytes, &vocabulary); e != nil {
		panic(e)
	}
	out := reference{ScannerSHA256: digest(scanner), Sources: map[string]string{}, VocabularySHA256: digest(vocabBytes), Methods: map[string]binding{}}
	entries, e := os.ReadDir(brainDir)
	if e != nil {
		panic(e)
	}
	for _, entry := range entries {
		n := entry.Name()
		if !entry.IsDir() && strings.HasSuffix(n, ".go") && !strings.HasSuffix(n, "_test.go") {
			out.Sources["services/hub/cmd/brain/"+n] = digest(read(filepath.Join(brainDir, n)))
		}
	}
	for method, params := range parseBrain(&scanError{}).brainMethodParams(&scanError{}) {
		b := binding{Fields: []string{}, Dangerous: []string{}, Opaque: params.opaque}
		for key := range params.keys {
			b.Fields = append(b.Fields, key)
			for name := range vocabulary.Params {
				if strings.EqualFold(name, key) {
					b.Dangerous = append(b.Dangerous, key)
					out.DangerousBindings++
					break
				}
			}
		}
		sort.Strings(b.Fields)
		sort.Strings(b.Dangerous)
		out.Methods[method] = b
	}
	if out.DangerousBindings != 84 {
		panic(fmt.Sprintf("original binding ratchet expected 84, observed %d", out.DangerousBindings))
	}
	encoded, e := json.MarshalIndent(out, "", "  ")
	if e != nil {
		panic(e)
	}
	encoded = append(encoded, '\n')
	if *check != "" {
		if !bytes.Equal(encoded, read(*check)) {
			panic("Go reference differs from capture")
		}
		fmt.Println("Go source provenance and 84 dangerous bindings match")
		return
	}
	os.Stdout.Write(encoded)
}
