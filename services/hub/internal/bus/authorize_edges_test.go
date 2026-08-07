package bus

// The wiring between the corpus predicate and the live gate.
//
// contracts/path-containment-cases.json drives pathWithinRoots DIRECTLY, and the
// paramShapes block asserts only paramString's `ok`. Everything between those two
// — what paramString RETURNS, what conn.authorize does with a resolution error,
// and what canonRoots produces for a root it cannot resolve — was covered by
// nothing, so three separate mutations each survived the whole internal/... tree:
//
//	paramString returning filepath.Clean(s)  → the symlink-plus-".." escape, back
//	authorize's `case err != nil` → nil      → relative/ELOOP/EACCES targets pass
//	canonRoots keeping a bad root verbatim   → an EMPTY root, which is a WILDCARD
//
// The last one is the reason `within` refuses an empty root outright: without
// that, `within("", "/etc/shadow")` takes the HasPrefix(target, "/") branch and
// is true for every absolute path on the system.

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
)

// paramString hands its VALUE to canonicalize, and canonicalize's own header
// forbids every whole-path helper because each of them collapses "link/.."
// textually BEFORE the link is read. Nothing asserted the value: the corpus calls
// pathWithinRoots directly, TestParamShapeContractCases writes `_, ok := …` and
// discards it, and the only authorize-level traversal case spells its target with
// filepath.Join, which has already Cleaned the string before the JSON is built.
func TestParamStringReturnsTheCallersStringVerbatim(t *testing.T) {
	for _, raw := range []string{
		"/root/link/../token",  // the escape: Clean() would collapse it here
		"/root/./a.txt",        // Clean() would drop the '.'
		"/root/a//b.txt",       // Clean() would collapse the double separator
		"/root/trailing/",      // Clean() would drop the trailing separator
		" /root/leading-space", // a filename may legitimately begin with a space
		"/root/sub/..",
	} {
		got, ok := paramString(json.RawMessage(`{"path":`+jstr(raw)+`}`), "path")
		if !ok {
			t.Errorf("paramString(%q) refused a well-formed string", raw)
			continue
		}
		if got != raw {
			t.Errorf("paramString normalized the caller's path: got %q, want %q verbatim — "+
				"any textual cleaning here collapses 'link/..' before the walk reads the link", got, raw)
		}
	}
}

// The same hole, end to end through the live gate rather than the helper: a
// plugin granted fs.read on <root> asks for <root>/link/../token where
// <root>/link is a directory symlink to <outside>. Cleaning ANYWHERE between the
// JSON and canonicalize() authorizes a path that opens <outside>/token.
func TestAuthorizeRefusesASymlinkTraversalOutOfTheGrantedRoot(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("symlink semantics differ on Windows")
	}
	base := t.TempDir()
	root := filepath.Join(base, "root")
	outside := filepath.Join(base, "outside")
	for _, d := range []string{root, outside} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(outside, "token"), []byte("s3kr3t"), 0o600); err != nil {
		t.Fatal(err)
	}
	gateSymlink(t, outside, filepath.Join(root, "link"))
	canon, err := canonicalize(root)
	if err != nil {
		t.Fatal(err)
	}
	cn := &conn{caps: map[string]capGrant{"fs.read": {fsRoots: []string{canon}}}}

	// Built as a STRING, not with filepath.Join: Join runs Clean, which is what
	// hid this from every existing authorize case.
	escape := root + "/link/../token"
	if err := cn.authorize("fs.read", json.RawMessage(`{"path":`+jstr(escape)+`}`)); err == nil {
		t.Fatalf("authorize allowed %q, which opens %q — outside the only granted root",
			escape, filepath.Join(outside, "token"))
	}
	// The floor: the same shape that stays inside must still be allowed.
	inside := root + "/sub/../a.txt"
	if err := cn.authorize("fs.read", json.RawMessage(`{"path":`+jstr(inside)+`}`)); err != nil {
		t.Fatalf("authorize refused %q, which resolves inside the root: %v", inside, err)
	}
}

// conn.authorize's `case err != nil` arm. The corpus has cases for every input
// that makes canonicalize fail (relative, ELOOP, EACCES, ENOTDIR) but they all
// call pathWithinRoots directly — nothing pinned the WIRING that turns that error
// into a denial, so flipping the arm to `return nil` let all of them through, two
// of them plain relative paths the provider then resolves against the daemon's
// own working directory.
func TestAuthorizeDeniesEveryTargetItCannotResolve(t *testing.T) {
	base := t.TempDir()
	root := filepath.Join(base, "root")
	if err := os.MkdirAll(root, 0o755); err != nil {
		t.Fatal(err)
	}
	targets := []string{"../../../etc/passwd", "notes.txt", "~/.ssh/id_rsa", "~"}

	// The ELOOP target is the only input in this list that reaches
	// canonicalize's hop-limit arm, and it used to be added `if err == nil` —
	// so a host without symlink privilege dropped it and nothing said so. The
	// gate does not skip the test (the other targets are still worth running);
	// it makes the missing coverage a named failure instead of an absence.
	if runtime.GOOS != "windows" {
		if gateSymlinkOptional(t, filepath.Join(root, "loop"), filepath.Join(root, "loop")) {
			targets = append(targets, filepath.Join(root, "loop", "x"))
		}
	}
	if err := os.WriteFile(filepath.Join(root, "afile"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	targets = append(targets, filepath.Join(root, "afile", "x")) // ENOTDIR
	locked := filepath.Join(root, "locked")
	if err := os.MkdirAll(filepath.Join(locked, "sub"), 0o755); err == nil {
		if runtime.GOOS != "windows" && os.Geteuid() != 0 {
			if os.Chmod(locked, 0) == nil {
				t.Cleanup(func() { _ = os.Chmod(locked, 0o700) })
				targets = append(targets, filepath.Join(locked, "sub", "x")) // EACCES
			}
		}
	}

	canon, err := canonicalize(root)
	if err != nil {
		t.Fatal(err)
	}
	cn := &conn{caps: map[string]capGrant{"fs.read": {fsRoots: []string{canon}}}}
	for _, target := range targets {
		if err := cn.authorize("fs.read", json.RawMessage(`{"path":`+jstr(target)+`}`)); err == nil {
			t.Errorf("authorize allowed %q, which it cannot resolve — anything unverifiable must fail closed", target)
		}
	}
}

// canonRoots is the ONLY production code that applies the DISCARD rule, and its
// five corpus cases are satisfied by the LOADER, which does the discard itself
// before handing roots to pathWithinRoots. So what canonRoots returns was
// asserted by nothing — and its failure mode is not a denial but a UNIVERSAL
// grant, because an un-discarded bad root becomes "" and within("", p) used to be
// true for every absolute path.
func TestCanonRootsDiscardsWhatItCannotResolve(t *testing.T) {
	bad := []string{"", "   ", "relative/dir", "~", "~/projects", "./x", ".."}
	if got := canonRoots(bad, "p", "fs.read"); len(got) != 0 {
		t.Fatalf("canonRoots kept %v; every one of these must be DISCARDED (empty, whitespace-only, relative, or tilde — which nobody expands)", got)
	}

	// One good root among the bad ones survives, and the bad ones neither poison
	// it nor widen it.
	good := t.TempDir()
	canonGood, err := canonicalize(good)
	if err != nil {
		t.Fatal(err)
	}
	got := canonRoots(append(append([]string{}, bad...), good), "p", "fs.read")
	if len(got) != 1 || got[0] != canonGood {
		t.Fatalf("canonRoots(bad… + %q) = %v, want exactly [%q]", good, got, canonGood)
	}

	// And the property that makes the discard load-bearing: an empty root grants
	// NOTHING. Without this, one un-canonicalizable root in a manifest silently
	// promoted a scoped plugin token to whole-filesystem fs.read/fs.write.
	for _, p := range []string{"/etc/passwd", "/root/.ssh/id_rsa", string(filepath.Separator), canonGood} {
		if within("", p) {
			t.Errorf(`within("", %q) = true — the empty string is behaving as a wildcard root`, p)
		}
	}
	cn := &conn{caps: map[string]capGrant{"fs.read": {fsRoots: []string{""}}}}
	if err := cn.authorize("fs.read", json.RawMessage(`{"path":"/etc/shadow"}`)); err == nil {
		t.Error("authorize allowed /etc/shadow through an empty root")
	}
}

// RegisterPluginToken is where canonRoots actually runs, so the end-to-end shape:
// a grant whose declared roots are ALL unresolvable must grant nothing, and must
// not be indistinguishable from a grant with no roots at all in the wrong
// direction (that case is already fail-closed and stays so).
func TestAGrantWhoseRootsAllFailToResolveGrantsNothing(t *testing.T) {
	s := NewServer(nil)
	const tok = "tok-all-bad"
	s.RegisterPluginToken(tok, "p", []capspec.Grant{{
		Method:  "fs.read",
		FSRoots: []string{"", "   ", "relative/dir", "~"},
	}}, capspec.EventGrants{})
	ident, ok := s.lookupPluginToken(tok)
	if !ok {
		t.Fatal("token not registered")
	}
	for _, g := range ident.caps {
		for _, r := range g.fsRoots {
			if strings.TrimSpace(r) == "" {
				t.Fatalf("a grant kept an empty root: %q — that is a wildcard, not a scope", r)
			}
		}
	}
	cn := &conn{caps: ident.caps}
	if err := cn.authorize("fs.read", json.RawMessage(`{"path":"/etc/shadow"}`)); err == nil {
		t.Fatal("a grant whose roots were all unresolvable authorized /etc/shadow")
	}
}
