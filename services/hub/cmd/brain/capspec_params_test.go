package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"reflect"
	"regexp"
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

// brainDangerousParamFloor is how many (method, param) pairs this scan finds in
// capspec's vocabulary across the whole dispatch switch. It is a RATCHET, held
// by capspec.RatchetError: a drop means the scan went blind, a rise means the
// number below is stale. The value it replaced was `if flagged < 10` against a
// true count in the high thirties — a floor that let two thirds of the coverage
// evaporate without a word.
const brainDangerousParamFloor = 56

// assertParamFloor applies the shared ratchet rule to one scan's count.
func assertParamFloor(t *testing.T, scan string, observed, floor int) {
	t.Helper()
	if msg := capspec.RatchetError(scan, observed, floor); msg != "" {
		t.Error(msg)
	}
}

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
func (ps *paramScan) brainMethodParams(t *testing.T) map[string]boundParams {
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
		for _, param := range sortedKeys(byMethod[method].keys) {
			// FOLD, not exact: encoding/json binds {"env": …} to a field tagged
			// `Env` and to an untagged `Env` alike, so an exact-spelling lookup
			// reported "not in the vocabulary" for both and nobody demanded a
			// decision. The canonical spelling is what the decision tables are
			// keyed by.
			_, kind, dangerous := capspec.DangerousKindFold(param)
			if !dangerous {
				// Not in the vocabulary is only reassuring if the name does not LOOK
				// like an executable, a path or a destination. The vocabulary is a
				// denylist and `entrypoint`, `exe` and `launcher` are all argv[0]:
				// the shape heuristic is what stops "nobody thought of that spelling"
				// from being a pass.
				if capspec.SuspiciousUnknownParam(param) {
					t.Errorf("the brain's %s handler binds %q, a name shaped like an executable/path/argv/destination that capspec's vocabulary does not know — so NO scan will ever demand a decision for it. Add it to dangerousParams with a kind, or to knownInertParams with the reason its shape is a coincidence.", method, param)
				}
				continue
			}
			flagged++
			if status, _, spelling := capspec.ClassifyParamFold(method, param); status == capspec.ParamUnclassified {
				t.Errorf("the brain's %s handler binds %q (a %s in capspec's vocabulary, consulted as %q) from caller params, and capspec classifies that PARAM nowhere — the decisions on record for %q are %v. Add a ParamDecision (kind + why), scope it in PathParam, or stop binding it.",
					method, param, kind, spelling, method, capspec.ParamDecisions(method))
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
		// The map[string]any shape: no struct, no json tag, just string literals
		// indexed out of the payload. Both of these scanned to an EMPTY list.
		{"layouts.save", "name"},
		{"sessions.save", "name"},
	} {
		if !byMethod[c.method].keys[c.param] {
			t.Errorf("the brain scan did not find %q on %s — the AST walk has stopped resolving a whole shape of handler, so every capability written that way is now unscanned", c.param, c.method)
		}
	}
	assertParamFloor(t, "brain dispatch", flagged, brainDangerousParamFloor)
}

// TestBrainOpaquePayloadHandlersAreClassified covers the shape no name-based
// scan can ever reach: `var partial map[string]any` unmarshalled straight from
// the caller's bytes and handed on whole, without a single key being spelled.
// config.save is written that way, and two of the keys inside that payload are
// process identifiers — agents.binaries is argv[0] of every spawned agent, and
// claude.profiles carries CLAUDE_CONFIG_DIR and --dangerously-skip-permissions.
// The params scan reported an EMPTY list for it and passed.
//
// There is nothing to flag by name here, so the requirement is one level up: a
// method that swallows the whole payload must have SOME classification on
// record — decisions (whose keys are then held to the host-trusted corpus by
// capspec's TestConfigSaveDecisionsMatchTheHostTrustedContract) or a PathParam
// entry. Silence is what a fail-open looks like.
// TestInertPathBearingMethodsBindNothing is the STRUCTURAL half of the door
// MissingSpec opened when it learned to treat an inert record as a
// classification.
//
// MissingSpec is the bus's fail-closed test: RegisterPluginToken continues on it
// and authorize() denies on it, so a method it calls "specced" is grantable with
// NO filesystem confinement. It consults inertMethods because providers.checkAll
// takes no params at all yet looks path-bearing by name. The cost is a third map
// somebody can file a real path method into.
//
// capspec's own TestInertPathBearingMethodsTakeNoParams guards that door by
// requiring the recorded reason to contain "no params" — and a reason is prose.
// Verified: filing `fs.exfiltrate` as inert with the reason "no params; it
// returns the provider's own already-loaded index …" satisfies that check while
// being false, and MissingSpec then reports the method specced, so the bus grants
// it unconfined with every package green. A guard defeated by writing the right
// sentence is not a guard.
//
// So this asserts the claim against the SCANNER instead of against the prose: an
// inert method under a path-verb prefix must bind nothing from the caller, as
// observed by the same AST walk that drives every other classification test.
// providers.checkAll passes because it genuinely binds nothing; the injected
// fs.exfiltrate would fail the moment it bound a single field.
func TestInertPathBearingMethodsBindNothing(t *testing.T) {
	ps := parseBrain(t)
	byMethod := ps.brainMethodParams(t)

	checked := 0
	for _, method := range capspec.InertMethods() {
		if !capspec.LooksPathBearing(method) {
			continue
		}
		checked++
		bound := byMethod[method]
		if len(bound.keys) > 0 {
			reason, _ := capspec.InertReason(method)
			t.Errorf("%q is on the inert record under a path-verb prefix, so MissingSpec reports it specced and the bus would grant it with NO path confinement — but the brain's handler binds %v from caller params. Its recorded reason (%q) is prose and cannot be checked; this can. Give it a PathParam entry, or stop binding caller values.",
				method, sortedKeys(bound.keys), reason)
		}
		if bound.opaque {
			t.Errorf("%q is inert under a path-verb prefix but unmarshals the caller's ENTIRE payload, so no scan can see what it carries. Inert cannot be claimed for a handler nobody can inspect.", method)
		}
	}
	if checked == 0 {
		t.Fatal("no inert method sits under a path-verb prefix — either pathVerbPrefixes or inertMethods changed and this guard now guards nothing (it exists because providers.checkAll is exactly that shape)")
	}
}

// TestInertMethodsAreActuallyRegistered closes the OTHER half of the same door.
//
// The scanner check above can only speak about a method some provider actually
// serves. A PHANTOM inert entry — a name no provider registers — binds nothing,
// so it passes every param scan while still making MissingSpec report it specced.
// That is exactly the injection that survived: `fs.exfiltrate` filed as inert with
// a plausible reason, no handler anywhere, and the bus grants it unconfined the
// day somebody implements it. The hole is opened by the entry and collected later.
//
// So an inert record may only name a capability that exists NOW, on some provider.
// There are THREE registration doors, and the first draft of this test knew two:
// it failed on notify.post, push.key and push.list, which are real capabilities
// registered by cmd/hub itself via RegisterLocal/RegisterLocalIdent. That is worth
// recording — the hub's own in-process registry is invisible to both the brain's
// registry and to any scan of hubCapabilities.ts, so it is the door a completeness
// check is most likely to forget.
var hubLocalRe = regexp.MustCompile(`(?m)\bRegisterLocal(?:Ident)?\(\s*"([a-zA-Z][\w.]*)"`)

func TestInertMethodsAreActuallyRegistered(t *testing.T) {
	registered := brainMethodSet()
	body := readDesktopCapabilities(t)
	for _, m := range names(catRe, body) {
		registered[m] = true
	}
	for _, m := range names(regRe, body) {
		registered[m] = true
	}
	hubMain := mustReadRepoFile(t, "services", "hub", "cmd", "hub", "main.go")
	hubLocal := names(hubLocalRe, string(hubMain))
	if len(hubLocal) == 0 {
		t.Fatal("no RegisterLocal capability found in cmd/hub/main.go — the hub's in-process registry moved or was renamed, and this guard would now silently under-count the registered set (which is a FALSE FAILURE direction, but it means the third door is unwatched)")
	}
	for _, m := range hubLocal {
		registered[m] = true
	}

	for _, m := range capspec.InertMethods() {
		if !registered[m] {
			reason, _ := capspec.InertReason(m)
			t.Errorf("capspec files %q as inert (%q) but NO provider registers it — not the brain's registry, not hubCapabilities.ts by cat() or registerCapability(). An inert record on a name nobody serves still makes MissingSpec report it specced, so the bus would grant it with no confinement the moment it is implemented. Remove the entry, or classify the capability that actually exists.",
				m, reason)
		}
	}
}

func TestBrainOpaquePayloadHandlersAreClassified(t *testing.T) {
	ps := parseBrain(t)
	byMethod := ps.brainMethodParams(t)

	opaque := 0
	for _, method := range sortedKeys(boolSet(byMethod)) {
		if !byMethod[method].opaque {
			continue
		}
		opaque++
		if _, scoped := capspec.IsPathScoped(method); scoped {
			continue
		}
		if len(capspec.ParamDecisions(method)) == 0 {
			t.Errorf("the brain's %s handler unmarshals the caller's ENTIRE payload into a map and never names a key, so no param scan can see anything it carries — and capspec records no decision for it either. Classify what the payload may contain (as config.save's keys are), or bind the fields you actually read.", method)
		}
	}
	if opaque == 0 {
		t.Error("the scan found no whole-payload map handler at all — config.save is one, so either the handler was rewritten or the map-target detection broke, and the shape that is invisible to every name-based check is now unwatched")
	}
}

// boolSet adapts the method map to the shared sortedKeys helper.
func boolSet(m map[string]boundParams) map[string]bool {
	out := make(map[string]bool, len(m))
	for k := range m {
		out[k] = true
	}
	return out
}
