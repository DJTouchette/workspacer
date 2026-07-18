package bus

import (
	"encoding/json"
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// mustCanon canonicalizes a root the way grant registration does, so tests
// compare canonical-to-canonical exactly like the live path.
func mustCanon(t *testing.T, p string) string {
	t.Helper()
	c, err := canonicalize(p)
	if err != nil {
		t.Fatalf("canonicalize(%q): %v", p, err)
	}
	return c
}

func TestPathWithinRoots(t *testing.T) {
	base := t.TempDir()
	root := filepath.Join(base, "project")
	sub := filepath.Join(root, "src")
	outside := filepath.Join(base, "secrets")
	for _, d := range []string{root, sub, outside} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(sub, "main.go"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(outside, "creds"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}

	canonRoot := mustCanon(t, root)

	cases := []struct {
		name   string
		target string
		want   bool
	}{
		{"existing file inside root", filepath.Join(sub, "main.go"), true},
		{"exact root", root, true},
		{"nested dir inside root", sub, true},
		{"new file under existing subdir (write target)", filepath.Join(sub, "new.txt"), true},
		{"new file via new nested dirs inside root", filepath.Join(root, "a", "b", "c.txt"), true},
		{"traversal escapes root", filepath.Join(root, "..", "secrets", "creds"), false},
		{"absolute path outside root", filepath.Join(outside, "creds"), false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got, err := pathWithinRoots([]string{canonRoot}, c.target)
			if err != nil {
				t.Fatalf("pathWithinRoots error: %v", err)
			}
			if got != c.want {
				t.Errorf("pathWithinRoots(%q) = %v, want %v", c.target, got, c.want)
			}
		})
	}
}

// A symlink inside the root that points outside must not let a target reached
// through it escape — canonicalize resolves the link before the prefix check.
func TestPathWithinRoots_SymlinkEscape(t *testing.T) {
	if runtime.GOOS == "windows" {
		t.Skip("symlink semantics differ on Windows")
	}
	base := t.TempDir()
	root := filepath.Join(base, "project")
	outside := filepath.Join(base, "secrets")
	for _, d := range []string{root, outside} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.WriteFile(filepath.Join(outside, "creds"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	// root/escape -> ../secrets
	link := filepath.Join(root, "escape")
	if err := os.Symlink(outside, link); err != nil {
		t.Fatal(err)
	}

	canonRoot := mustCanon(t, root)
	// Reading "inside" the root but through the symlink lands on the secret.
	got, err := pathWithinRoots([]string{canonRoot}, filepath.Join(link, "creds"))
	if err != nil {
		t.Fatalf("pathWithinRoots error: %v", err)
	}
	if got {
		t.Errorf("symlink escape was allowed: %q resolved inside %q", filepath.Join(link, "creds"), canonRoot)
	}
}

// A root whose name is a string prefix of a sibling must not capture the sibling.
func TestWithin_SiblingPrefixIsNotContained(t *testing.T) {
	root := filepath.FromSlash("/srv/foo")
	if within(root, filepath.FromSlash("/srv/foobar/x")) {
		t.Errorf("sibling /srv/foobar wrongly treated as inside /srv/foo")
	}
	if !within(root, filepath.FromSlash("/srv/foo/x")) {
		t.Errorf("/srv/foo/x should be inside /srv/foo")
	}
	if !within(root, root) {
		t.Errorf("root should contain itself")
	}
}

func TestPathWithinRoots_MultipleRoots(t *testing.T) {
	base := t.TempDir()
	a := filepath.Join(base, "a")
	b := filepath.Join(base, "b")
	other := filepath.Join(base, "c")
	for _, d := range []string{a, b, other} {
		if err := os.MkdirAll(d, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	roots := []string{mustCanon(t, a), mustCanon(t, b)}

	if ok, _ := pathWithinRoots(roots, filepath.Join(b, "f.txt")); !ok {
		t.Errorf("target under second root should be allowed")
	}
	if ok, _ := pathWithinRoots(roots, filepath.Join(other, "f.txt")); ok {
		t.Errorf("target under an ungranted root should be denied")
	}
}

func TestParamString(t *testing.T) {
	cases := []struct {
		name   string
		params string
		field  string
		want   string
		ok     bool
	}{
		{"present", `{"path":"/a/b"}`, "path", "/a/b", true},
		{"cwd field", `{"cwd":"/proj","query":"x"}`, "cwd", "/proj", true},
		{"absent", `{"query":"x"}`, "path", "", false},
		{"empty string", `{"path":""}`, "path", "", false},
		{"wrong type", `{"path":123}`, "path", "", false},
		{"malformed", `{not json`, "path", "", false},
		{"empty params", ``, "path", "", false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got, ok := paramString(json.RawMessage(c.params), c.field)
			if got != c.want || ok != c.ok {
				t.Errorf("paramString(%s, %q) = (%q, %v), want (%q, %v)", c.params, c.field, got, ok, c.want, c.ok)
			}
		})
	}
}

// TestWithin_FilesystemRootContainsEverything covers a grant whose canonical
// root is the filesystem root ("/"): it declares the whole tree, so within()
// must treat every absolute path as contained. Concatenating an extra separator
// ("//") would match nothing, silently denying the grant. Covers idx 20.
func TestWithin_FilesystemRootContainsEverything(t *testing.T) {
	root := string(os.PathSeparator) // "/" on Unix, "\" on Windows
	target := filepath.Join(root, "home", "u", "f")
	if !within(root, target) {
		t.Errorf("root %q (whole filesystem) should contain %q", root, target)
	}
	if !within(root, filepath.Join(root, "etc", "x")) {
		t.Errorf("root %q should contain %q", root, filepath.Join(root, "etc", "x"))
	}
}
