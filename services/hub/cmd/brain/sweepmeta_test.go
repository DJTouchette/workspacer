package main

import (
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/extinput"
	"github.com/djtouchette/workspacer-hub/internal/sweepguard"
)

// ---------------------------------------------------------------------------
// THE GUARD'S OWN GUARD.
//
// Everything sweepguard and sweepTally.ts add is a floor, and a floor is a piece
// of bookkeeping that is trivially satisfiable by NOT DOING IT:
//
//   - Build a Tally, count into it, never call Require. The sweep is back to
//     reporting nothing, with more code. (This shipped: libraryService.test.ts
//     constructed a GateCounter around four tests, incremented it, and never
//     read it — the four tests were the entire oracle for "the leg opens the
//     path the guard resolved", and on a host without symlink privilege all four
//     were `it.skip` inside a green file.)
//   - Call Ran from the loop that REGISTERS subtests instead of from the body.
//     The count is then of enumeration, and reports a full house for a run in
//     which every case skipped. (This shipped too, in the very sweep whose
//     header describes the run where all eight of its subtests skipped:
//     fsguard_test.go's `ran++` sat next to t.Run.)
//   - Declare a host gate and never assert it ran.
//
// So the rules are enforced mechanically, over BOTH stacks, from one place. A
// meta-guard is only worth having if it is cheap to obey and impossible to
// forget, which is why it names the file, the line and the fix.
// ---------------------------------------------------------------------------

// sweptDirs are the test trees this guard reads. Go and TypeScript both, because
// the rules are the same rules and the TypeScript side is where the unread
// counter was found.
var sweptGoDirs = []string{
	filepath.Join("services", "hub", "cmd", "brain"),
	filepath.Join("services", "hub", "internal", "bus"),
}

var sweptTSDirs = []string{
	filepath.Join("apps", "desktop", "src", "main"),
}

// TestEverySweepCounterIsAssertedOn is rule 1: a counter nobody reads is a
// comment.
func TestEverySweepCounterIsAssertedOn(t *testing.T) {
	checked := 0

	// --- Go: every sweepguard.Tally must be Require*'d in its own function ---
	for _, dir := range sweptGoDirs {
		for _, file := range goTestFiles(t, dir) {
			fset := token.NewFileSet()
			f, err := parser.ParseFile(fset, file, nil, parser.ParseComments)
			if err != nil {
				t.Fatalf("parse %s: %v", file, err)
			}
			ast.Inspect(f, func(n ast.Node) bool {
				fn, ok := n.(*ast.FuncDecl)
				if !ok || fn.Body == nil {
					return true
				}
				for name, pos := range tallyNames(fn) {
					checked++
					if !hasRequireCall(fn, name) {
						t.Errorf("%s:%d: %s builds sweepguard.Tally %q and never calls Require/RequireBoth/RequireCorpus/RequireEvery/RequireDeny on it — a tally nobody reads counts cases into a variable and asserts nothing, which is the shape libraryService.test.ts shipped",
							relTo(t, file), fset.Position(pos).Line, fn.Name.Name, name)
					}
				}
				return true
			})
		}
	}

	// --- TypeScript: every `new SweepTally()` must reach an itSwept* floor ---
	tallyDecl := regexp.MustCompile(`const\s+(\w+)\s*=\s*new SweepTally\(\)`)
	gateUse := regexp.MustCompile(`gatedIt\([^,]+,\s*(\w+)\s*\)`)
	for _, file := range tsTestFiles(t, sweptTSDirs) {
		src := readText(t, file)
		for _, m := range tallyDecl.FindAllStringSubmatch(src, -1) {
			checked++
			name := m[1]
			if !regexp.MustCompile(`itSwept\w*\(\s*` + regexp.QuoteMeta(name) + `\b`).MatchString(src) {
				t.Errorf("%s: `new SweepTally()` assigned to %q with no itSweptBothVerdicts/itSweptTheWholeCorpus/itSweptAtLeast(%s, ...) anywhere in the file — the tally counts and nothing reads it",
					relTo(t, file), name, name)
			}
		}
		for _, m := range gateUse.FindAllStringSubmatch(src, -1) {
			checked++
			name := m[1]
			if !regexp.MustCompile(`itRanEveryGatedTest\(\s*` + regexp.QuoteMeta(name) + `\b`).MatchString(src) {
				t.Errorf("%s: gatedIt(..., %s) with no itRanEveryGatedTest(%s, ...) in the file — the group's tests can all become `it.skip` with the file still green. This is the exact defect that shipped here.",
					relTo(t, file), name, name)
			}
		}
	}

	if checked < sweepCounterFloor {
		t.Fatalf("this meta-guard found only %d counters across both stacks (floor %d) — it is reading the wrong directories, or the machinery it polices has been removed", checked, sweepCounterFloor)
	}
	t.Logf("checked %d sweep counters", checked)
}

// sweepCounterFloor is how many Tally/GateCounter uses this guard finds today.
// Without it, a meta-guard whose file globs stopped matching would pass by
// finding nothing — the same "guarded nothing, printed ok" failure one level up.
const sweepCounterFloor = 30

// TestSweepCountersAreIncrementedFromTheBody is rule 2, and it is the rule the
// original defect broke: sweepguard's own doc says Ran is called from inside the
// subtest body, past every skip gate, and never from the loop that registers
// subtests.
func TestSweepCountersAreIncrementedFromTheBody(t *testing.T) {
	for _, dir := range sweptGoDirs {
		for _, file := range goTestFiles(t, dir) {
			fset := token.NewFileSet()
			f, err := parser.ParseFile(fset, file, nil, parser.ParseComments)
			if err != nil {
				t.Fatalf("parse %s: %v", file, err)
			}
			ast.Inspect(f, func(n ast.Node) bool {
				body := loopBody(n)
				if body == nil || !containsSubtestRegistration(body) {
					return true
				}
				// Statements at the loop's OWN level — anything inside a func
				// literal (which is what t.Run takes) is a body, not the loop.
				for _, stmt := range body.List {
					if lit := containsFuncLit(stmt); lit {
						continue
					}
					if pos, what := countingStatement(stmt); pos.IsValid() {
						t.Errorf("%s:%d: %s sits in a loop that registers subtests, OUTSIDE the subtest body — that counts REGISTRATION, and a registration count reports a full house for a run in which every subtest skipped. Move it inside the t.Run closure, past every skip gate. (sweepguard's package doc, first paragraph.)",
							relTo(t, file), fset.Position(pos).Line, what)
					}
				}
				return true
			})
		}
	}

	// TypeScript: `x.ran(...)` must be inside an it()/itLinks() callback, not in
	// the `for (const c of fixture.cases)` that registers them.
	for _, file := range tsTestFiles(t, sweptTSDirs) {
		src := readText(t, file)
		for _, idx := range regexp.MustCompile(`\w+\.ran\(`).FindAllStringIndex(src, -1) {
			if opener := nearestOpener(src[:idx[0]]); opener == "for" {
				t.Errorf("%s:%d: a `.ran(` call whose nearest enclosing block is a `for` and not an `it(` callback — that counts registration, not execution",
					relTo(t, file), 1+strings.Count(src[:idx[0]], "\n"))
			}
		}
	}
}

// TestNoUncountedHostGateInTypeScriptTests is rule 4, and it closes a hole in
// this guard's VOCABULARY rather than in its wiring.
//
// Rules 1–3 key on the literal tokens `new SweepTally()`, `gatedIt(X, name)`,
// `.ran(` and `sweepguard.Gate`. A group that uses none of them is structurally
// invisible: reverting a gated group to the pre-round-3 idiom
//
//	const itLinks = CAN_SYMLINK ? it : it.skip;   // and delete the floor line
//
// left cmd/brain's three meta-guard tests reporting `ok`, while the group they
// were meant to protect became "silently skipped on a host without symlink
// privilege" again. The two positive controls DO bite (deleting a
// tally.RequireCorpus, or a floor line next to a surviving `gatedIt`), so the
// gap was the set of shapes recognised, not the mechanism.
//
// It is not hypothetical. pathConfinement.test.ts's resolveStoreEntry group
// shipped in the OTHER unrecognised shape — `try { fs.symlinkSync(...) } catch {
// return }` — and reported three green ticks on a host that could not make a
// symlink, while being the only oracle for the fix it was written for.
//
// So both shapes are banned by name, with the accounted alternatives spelled
// out in the failure message. Comments are stripped first: several of these
// files QUOTE the banned idiom in prose explaining why it is banned, and a guard
// that fires on its own documentation is the joke the rest of this file exists
// to stop telling.
func TestNoUncountedHostGateInTypeScriptTests(t *testing.T) {
	// A catch whose entire body is a bare `return`: the test abandons itself and
	// vitest records a PASS.
	swallow := regexp.MustCompile(`catch\s*(\([^)]*\))?\s*\{\s*return\s*;?\s*\}`)
	// Any reference to a host-capability constant.
	hostCap := regexp.MustCompile(`\bCAN_[A-Z0-9_]+\b`)
	// The three accounted ways to consume one.
	viaGatedIt := regexp.MustCompile(`gatedIt\(`)
	viaAlias := regexp.MustCompile(`^\s*(export\s+)?const\s+CAN_[A-Z0-9_]+\s*(:[^=]+)?=`)
	// A skip-REASON producer feeds a SweepTally through `tally.skip(reason)`;
	// the fixture's own vocabulary is the tell.
	viaSkipReason := regexp.MustCompile(`needs[A-Z]\w*`)
	// An import naming the constant is not a use of it.
	viaImport := regexp.MustCompile(`^\s*(import\b|.*\bfrom\s+'|CAN_[A-Z0-9_]+,\s*$)`)

	checked := 0
	for _, file := range tsTestFiles(t, sweptTSDirs) {
		src := stripTSComments(readText(t, file))
		for _, idx := range swallow.FindAllStringIndex(src, -1) {
			t.Errorf("%s:%d: `catch { return }` swallows a setup failure and vitest reports the test as PASSED. If the setup needs a host privilege, register the test with gatedIt(CAN_X, counter) and assert the group with itRanEveryGatedTest — a host that cannot run the test must be RED, not green with a tick.",
				relTo(t, file), 1+strings.Count(src[:idx[0]], "\n"))
		}
		for _, line := range strings.Split(src, "\n") {
			if !hostCap.MatchString(line) {
				continue
			}
			checked++
			if viaGatedIt.MatchString(line) || viaAlias.MatchString(line) ||
				viaSkipReason.MatchString(line) || viaImport.MatchString(line) {
				continue
			}
			t.Errorf("%s: a host-capability constant is consumed by an UNCOUNTED gate:\n    %s\n  Only three shapes are accounted for: `gatedIt(CAN_X, counter)` (+ itRanEveryGatedTest), an alias declaration, or a skip-REASON producer feeding a SweepTally. `CAN_X ? it : it.skip` is none of them — it makes the group skippable inside a green file, which is the defect this whole family of guards exists to stop.",
				relTo(t, file), strings.TrimSpace(line))
		}
	}
	if checked < hostCapUseFloor {
		t.Fatalf("this rule found only %d host-capability references (floor %d) — either the constants were renamed or it is reading the wrong directories, and a guard that finds nothing passes", checked, hostCapUseFloor)
	}
	t.Logf("checked %d host-capability references", checked)
}

// hostCapUseFloor is how many CAN_* references the rule above finds today.
const hostCapUseFloor = 20

// stripTSComments blanks `//` and `/* */` comments, preserving every byte
// position (and therefore every line number) by writing spaces in their place.
// String and template literals are tracked so a `//` inside one is not mistaken
// for a comment.
func stripTSComments(src string) string {
	out := []byte(src)
	const (
		code = iota
		line
		block
		sq
		dq
		tick
	)
	state := code
	for i := 0; i < len(src); i++ {
		c := src[i]
		switch state {
		case code:
			switch {
			case c == '/' && i+1 < len(src) && src[i+1] == '/':
				state, out[i], out[i+1] = line, ' ', ' '
				i++
			case c == '/' && i+1 < len(src) && src[i+1] == '*':
				state, out[i], out[i+1] = block, ' ', ' '
				i++
			case c == '\'':
				state = sq
			case c == '"':
				state = dq
			case c == '`':
				state = tick
			}
		case line:
			if c == '\n' {
				state = code
			} else {
				out[i] = ' '
			}
		case block:
			if c == '*' && i+1 < len(src) && src[i+1] == '/' {
				state, out[i], out[i+1] = code, ' ', ' '
				i++
			} else if c != '\n' {
				out[i] = ' '
			}
		case sq, dq, tick:
			if c == '\\' {
				i++
				continue
			}
			if (state == sq && c == '\'') || (state == dq && c == '"') || (state == tick && c == '`') {
				state = code
			}
		}
	}
	return string(out)
}

// TestEveryHostGateIsCheckedByTestMain is rule 3. A Go GateCounter has no
// per-test floor of its own: RunGates is what reads it, from TestMain, so that a
// package cannot report ok while a whole gated group skipped. A package that
// declares a gate and forgets the TestMain has a counter nobody reads.
func TestEveryHostGateIsCheckedByTestMain(t *testing.T) {
	found := 0
	for _, dir := range sweptGoDirs {
		files := goTestFiles(t, dir)
		declaresGate, hasRunGates := false, false
		for _, file := range files {
			// PARSED, not grepped. This test's own error strings contain the
			// words "sweepguard.RunGates" and "func TestMain", so a substring
			// check is satisfied by the file complaining that the call is
			// missing — a guard that passes because of its own failure message,
			// which is the joke this whole run exists to stop telling. (Verified
			// by deleting the real TestMain: the grep version stayed green.)
			fset := token.NewFileSet()
			f, err := parser.ParseFile(fset, file, nil, 0)
			if err != nil {
				t.Fatalf("parse %s: %v", file, err)
			}
			ast.Inspect(f, func(n ast.Node) bool {
				if call, ok := n.(*ast.CallExpr); ok && isSweepguardCall(call, "Gate") {
					declaresGate = true
				}
				fn, ok := n.(*ast.FuncDecl)
				if !ok || fn.Name.Name != "TestMain" || fn.Body == nil {
					return true
				}
				ast.Inspect(fn.Body, func(inner ast.Node) bool {
					if call, ok := inner.(*ast.CallExpr); ok && isSweepguardCall(call, "RunGates") {
						hasRunGates = true
					}
					return true
				})
				return true
			})
		}
		if !declaresGate {
			t.Errorf("%s declares no sweepguard.Gate at all — every package in this list has host-gated tests (symlinks, git), and a package without a gate is one where those tests can all skip inside a green run", dir)
			continue
		}
		found++
		if !hasRunGates {
			t.Errorf("%s declares a sweepguard.Gate but has no `func TestMain` calling sweepguard.RunGates — nothing reads the gate, so the group can empty itself and the package still prints ok", dir)
		}
	}
	if found != len(sweptGoDirs) {
		t.Fatalf("only %d of %d Go test packages carry a host gate", found, len(sweptGoDirs))
	}
}

// isSweepguardCall reports whether a call is sweepguard.<name>(...).
func isSweepguardCall(call *ast.CallExpr, name string) bool {
	sel, ok := call.Fun.(*ast.SelectorExpr)
	if !ok || sel.Sel.Name != name {
		return false
	}
	pkg, ok := sel.X.(*ast.Ident)
	return ok && pkg.Name == "sweepguard"
}

// --- helpers ---------------------------------------------------------------

func goTestFiles(t *testing.T, dir string) []string {
	t.Helper()
	root, err := sweepguard.RepoPath(dir)
	if err != nil {
		t.Fatalf("locate %s: %v", dir, err)
	}
	entries, err := extinput.ReadDir(root)
	if err != nil {
		t.Fatalf("read %s: %v", root, err)
	}
	var out []string
	for _, e := range entries {
		if !e.IsDir() && strings.HasSuffix(e.Name(), "_test.go") {
			out = append(out, filepath.Join(root, e.Name()))
		}
	}
	if len(out) == 0 {
		t.Fatalf("%s holds no _test.go files — this meta-guard is reading the wrong place", root)
	}
	sort.Strings(out)
	return out
}

func tsTestFiles(t *testing.T, dirs []string) []string {
	t.Helper()
	var out []string
	for _, dir := range dirs {
		root, err := sweepguard.RepoPath(dir)
		if err != nil {
			t.Fatalf("locate %s: %v", dir, err)
		}
		// walkPinned, not filepath.WalkDir: apps/desktop is outside this Go
		// module, and cmd/go re-hashes only what a test OPENED through a path
		// that lexically descends from the module root. readText below pins the
		// CONTENTS it reads, but a plain WalkDir leaves the LISTING unpinned —
		// so a NEW .test.ts carrying an unasserted tally, which is precisely
		// what this meta-guard exists to catch, would arrive to `ok (cached)`.
		if err := walkPinned(root, func(path string, d os.DirEntry) error {
			if !d.IsDir() && strings.HasSuffix(path, ".test.ts") {
				out = append(out, path)
			}
			return nil
		}); err != nil {
			t.Fatalf("walk %s: %v", root, err)
		}
	}
	if len(out) == 0 {
		t.Fatal("no .test.ts files found — this meta-guard is reading the wrong place")
	}
	sort.Strings(out)
	return out
}

func readText(t *testing.T, path string) string {
	t.Helper()
	// Through extinput's reader, like every other cross-repo read here: files
	// above the module root are not in cmd/go's test-cache key when read with a
	// path that escapes it, so a plain os.ReadFile leaves this guard printing
	// `ok (cached)` over bytes it never looked at.
	rel, err := filepath.Rel(mustRepoRoot(t), path)
	if err != nil {
		t.Fatalf("rel %s: %v", path, err)
	}
	data, err := sweepguard.ReadRepoFile(strings.Split(filepath.ToSlash(rel), "/")...)
	if err != nil {
		t.Fatalf("read %s: %v", path, err)
	}
	return string(data)
}

func mustRepoRoot(t *testing.T) string {
	t.Helper()
	root, err := sweepguard.Root()
	if err != nil {
		t.Fatalf("repo root: %v", err)
	}
	return root
}

func relTo(t *testing.T, path string) string {
	t.Helper()
	rel, err := filepath.Rel(mustRepoRoot(t), path)
	if err != nil {
		return path
	}
	return filepath.ToSlash(rel)
}

// tallyNames returns the sweepguard.Tally variables a function declares.
func tallyNames(fn *ast.FuncDecl) map[string]token.Pos {
	out := map[string]token.Pos{}
	ast.Inspect(fn, func(n ast.Node) bool {
		switch d := n.(type) {
		case *ast.DeclStmt:
			gen, ok := d.Decl.(*ast.GenDecl)
			if !ok || gen.Tok != token.VAR {
				return true
			}
			for _, spec := range gen.Specs {
				vs, ok := spec.(*ast.ValueSpec)
				if !ok || !isTallyType(vs.Type) {
					continue
				}
				for _, name := range vs.Names {
					out[name.Name] = name.Pos()
				}
			}
		case *ast.AssignStmt:
			for i, rhs := range d.Rhs {
				comp, ok := rhs.(*ast.CompositeLit)
				if !ok || !isTallyType(comp.Type) || i >= len(d.Lhs) {
					continue
				}
				if id, ok := d.Lhs[i].(*ast.Ident); ok {
					out[id.Name] = id.Pos()
				}
			}
		}
		return true
	})
	return out
}

func isTallyType(expr ast.Expr) bool {
	sel, ok := expr.(*ast.SelectorExpr)
	if !ok {
		return false
	}
	pkg, ok := sel.X.(*ast.Ident)
	return ok && pkg.Name == "sweepguard" && sel.Sel.Name == "Tally"
}

// hasRequireCall reports whether the function asserts on the named tally.
func hasRequireCall(fn *ast.FuncDecl, name string) bool {
	found := false
	ast.Inspect(fn, func(n ast.Node) bool {
		call, ok := n.(*ast.CallExpr)
		if !ok {
			return true
		}
		sel, ok := call.Fun.(*ast.SelectorExpr)
		if !ok || !strings.HasPrefix(sel.Sel.Name, "Require") {
			return true
		}
		switch recv := sel.X.(type) {
		case *ast.Ident:
			if recv.Name == name {
				found = true
			}
		case *ast.UnaryExpr: // &tally
			if id, ok := recv.X.(*ast.Ident); ok && id.Name == name {
				found = true
			}
		}
		return true
	})
	if found {
		return true
	}
	// A tally handed to a table of {name, *Tally, floor} is asserted in the loop
	// that walks the table; the address-of is the tell.
	ast.Inspect(fn, func(n ast.Node) bool {
		u, ok := n.(*ast.UnaryExpr)
		if !ok || u.Op != token.AND {
			return true
		}
		if id, ok := u.X.(*ast.Ident); ok && id.Name == name {
			found = true
		}
		return true
	})
	return found
}

func loopBody(n ast.Node) *ast.BlockStmt {
	switch l := n.(type) {
	case *ast.RangeStmt:
		return l.Body
	case *ast.ForStmt:
		return l.Body
	}
	return nil
}

func containsSubtestRegistration(body *ast.BlockStmt) bool {
	found := false
	ast.Inspect(body, func(n ast.Node) bool {
		call, ok := n.(*ast.CallExpr)
		if !ok {
			return true
		}
		if sel, ok := call.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == "Run" {
			if id, ok := sel.X.(*ast.Ident); ok && id.Name == "t" {
				found = true
			}
		}
		return true
	})
	return found
}

// containsFuncLit reports whether a statement is (or wraps) a closure — the
// subtest body itself, which is where counting BELONGS.
func containsFuncLit(stmt ast.Stmt) bool {
	found := false
	ast.Inspect(stmt, func(n ast.Node) bool {
		if _, ok := n.(*ast.FuncLit); ok {
			found = true
		}
		return true
	})
	return found
}

// countingStatement recognises the two shapes of "I am counting here": a
// `x.Ran(...)` call and a bare `n++`.
func countingStatement(stmt ast.Stmt) (token.Pos, string) {
	switch s := stmt.(type) {
	case *ast.IncDecStmt:
		if s.Tok == token.INC {
			if id, ok := s.X.(*ast.Ident); ok {
				return s.Pos(), "`" + id.Name + "++`"
			}
		}
	case *ast.ExprStmt:
		call, ok := s.X.(*ast.CallExpr)
		if !ok {
			return token.NoPos, ""
		}
		if sel, ok := call.Fun.(*ast.SelectorExpr); ok && (sel.Sel.Name == "Ran" || sel.Sel.Name == "Skip") {
			if id, ok := sel.X.(*ast.Ident); ok {
				return s.Pos(), "`" + id.Name + "." + sel.Sel.Name + "(...)`"
			}
		}
	case *ast.AssignStmt:
		// `covered[site] = true` beside a t.Run is the same lie in map form.
		if len(s.Lhs) == 1 && s.Tok == token.ASSIGN {
			if idx, ok := s.Lhs[0].(*ast.IndexExpr); ok {
				if id, ok := idx.X.(*ast.Ident); ok && strings.Contains(strings.ToLower(id.Name), "cover") {
					return s.Pos(), "`" + id.Name + "[...] = ...`"
				}
			}
		}
	}
	return token.NoPos, ""
}

// nearestOpener names the innermost unclosed block opener before an offset:
// "for", "it" or "" — a coarse but sufficient reading of TypeScript, since the
// only thing this rule has to tell apart is a registration loop from a test
// body.
var reForLoop = regexp.MustCompile(`\bfor\s*\(`)

func nearestOpener(before string) string {
	type frame struct{ kind string }
	var stack []frame
	lines := strings.Split(before, "\n")
	for _, line := range lines {
		code := line
		if i := strings.Index(code, "//"); i >= 0 {
			code = code[:i]
		}
		kind := ""
		if reForLoop.MatchString(code) {
			kind = "for"
		}
		opens := strings.Count(code, "{") + strings.Count(code, "(")
		closes := strings.Count(code, "}") + strings.Count(code, ")")
		for i := 0; i < opens-closes; i++ {
			stack = append(stack, frame{kind})
			kind = ""
		}
		// An arrow callback opening a block on this line is a test BODY, and it
		// is the innermost thing opened — `run(name, () => {` inside a
		// registration loop is the case the rule has to get right, and `run` is
		// an alias for `it`/`it.skip` in two of these files, so matching on the
		// callee name is not enough.
		if strings.Contains(code, "=> {") && len(stack) > 0 {
			stack[len(stack)-1] = frame{"it"}
		}
		for i := 0; i < closes-opens && len(stack) > 0; i++ {
			stack = stack[:len(stack)-1]
		}
	}
	for i := len(stack) - 1; i >= 0; i-- {
		if stack[i].kind != "" {
			return stack[i].kind
		}
	}
	return ""
}
