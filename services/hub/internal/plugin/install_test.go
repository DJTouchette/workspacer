package plugin

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

// makeTarGz builds a gzipped tar where every file is nested under a single
// top-level wrap dir, mimicking a GitHub source tarball.
func makeTarGz(t *testing.T, wrap string, files map[string]string) []byte {
	t.Helper()
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	for name, content := range files {
		full := wrap + "/" + name
		if err := tw.WriteHeader(&tar.Header{Name: full, Mode: 0o644, Size: int64(len(content)), Typeflag: tar.TypeReg}); err != nil {
			t.Fatal(err)
		}
		if _, err := tw.Write([]byte(content)); err != nil {
			t.Fatal(err)
		}
	}
	tw.Close()
	gz.Close()
	return buf.Bytes()
}

func serveTarball(t *testing.T, data []byte) string {
	t.Helper()
	hs := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Content-Type", "application/gzip")
		_, _ = w.Write(data)
	}))
	t.Cleanup(hs.Close)
	return hs.URL
}

func TestResolveTarballURLs(t *testing.T) {
	cases := []struct {
		in    string
		first string
		name  string
	}{
		{"owner/repo", "https://codeload.github.com/owner/repo/tar.gz/main", "repo"},
		{"https://github.com/owner/repo", "https://codeload.github.com/owner/repo/tar.gz/main", "repo"},
		{"https://github.com/owner/repo.git", "https://codeload.github.com/owner/repo/tar.gz/main", "repo"},
		{"github.com/owner/repo/tree/dev", "https://codeload.github.com/owner/repo/tar.gz/dev", "repo"},
		{"https://example.com/x.tar.gz", "https://example.com/x.tar.gz", ""},
	}
	for _, c := range cases {
		urls, name, err := resolveTarballURLs(c.in)
		if err != nil {
			t.Errorf("%q: %v", c.in, err)
			continue
		}
		if urls[0] != c.first || name != c.name {
			t.Errorf("%q → urls[0]=%q name=%q want %q/%q", c.in, urls[0], name, c.first, c.name)
		}
	}
	if _, _, err := resolveTarballURLs("not-a-repo"); err == nil {
		t.Error("expected error for bad input")
	}
}

func TestInstallFromTarballHappy(t *testing.T) {
	data := makeTarGz(t, "acme-clock-main", map[string]string{
		"plugin.json": `{"id":"acme.clock","name":"Clock","apiVersion":"1","server":{"command":"python3","args":["-m","http.server","9001"],"port":9001,"health":"/"},"panes":[{"type":"acme.clock","title":"Clock"}]}`,
		"index.html":  "<html>clock</html>",
	})
	url := serveTarball(t, data)
	dir := t.TempDir()

	m, err := installFromTarball(dir, url, "fallback", nil)
	if err != nil {
		t.Fatal(err)
	}
	if m.ID != "acme.clock" || len(m.Panes) != 1 {
		t.Fatalf("manifest = %+v", m)
	}
	// Installed under a sanitized name; files present; Dir set.
	want := filepath.Join(dir, "acme-clock")
	if m.Dir != want {
		t.Errorf("Dir = %q want %q", m.Dir, want)
	}
	for _, f := range []string{"plugin.json", "index.html"} {
		if _, err := os.Stat(filepath.Join(want, f)); err != nil {
			t.Errorf("missing %s: %v", f, err)
		}
	}
}

func TestInstallReinstallOverwrites(t *testing.T) {
	dir := t.TempDir()
	mk := func(html string) string {
		return serveTarball(t, makeTarGz(t, "r-main", map[string]string{
			"plugin.json": `{"id":"x.y","apiVersion":"1"}`,
			"index.html":  html,
		}))
	}
	if _, err := installFromTarball(dir, mk("v1"), "r", nil); err != nil {
		t.Fatal(err)
	}
	if _, err := installFromTarball(dir, mk("v2"), "r", nil); err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(filepath.Join(dir, "x-y", "index.html"))
	if string(got) != "v2" {
		t.Fatalf("reinstall did not overwrite: %q", got)
	}
}

func TestInstallRunsBuildCommand(t *testing.T) {
	data := makeTarGz(t, "b-main", map[string]string{
		"plugin.json": `{"id":"b.uild","apiVersion":"1","install":["sh","-c","echo done > built.marker"]}`,
	})
	dir := t.TempDir()
	if _, err := installFromTarball(dir, serveTarball(t, data), "b", nil); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, "b-uild", "built.marker")); err != nil {
		t.Fatalf("install command did not run: %v", err)
	}
}

func TestInstallNoManifest(t *testing.T) {
	data := makeTarGz(t, "empty-main", map[string]string{"readme.md": "hi"})
	dir := t.TempDir()
	if _, err := installFromTarball(dir, serveTarball(t, data), "empty", nil); err == nil {
		t.Error("expected error when archive has no plugin.json")
	}
}

func TestInstallFromDir(t *testing.T) {
	// A bundled-example-style source dir with a manifest + a ui asset.
	src := t.TempDir()
	if err := os.WriteFile(filepath.Join(src, "plugin.json"),
		[]byte(`{"id":"example.hello","name":"Hello","apiVersion":"1","ui":"ui","panes":[{"type":"example.hello","title":"Hello"}]}`), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.MkdirAll(filepath.Join(src, "ui"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(src, "ui", "index.html"), []byte("<html>hi</html>"), 0o644); err != nil {
		t.Fatal(err)
	}

	dir := t.TempDir()
	m, err := InstallFromDir(dir, src)
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(dir, "example-hello")
	if m.ID != "example.hello" || m.Dir != want {
		t.Fatalf("manifest = %+v (want Dir %q)", m, want)
	}
	// The whole tree (incl. the ui subdir) is copied into the plugins dir.
	if _, err := os.Stat(filepath.Join(want, "ui", "index.html")); err != nil {
		t.Errorf("ui asset not copied: %v", err)
	}
	// No .install-source for a bundled example (nothing to update from).
	if _, err := os.Stat(filepath.Join(want, sourceFile)); err == nil {
		t.Error("InstallFromDir should not write an .install-source")
	}

	// Re-adding overwrites cleanly.
	if _, err := InstallFromDir(dir, src); err != nil {
		t.Fatalf("re-add failed: %v", err)
	}
}

func TestStripPath(t *testing.T) {
	if got := stripPath("repo-main/sub/file.txt", 1); got != filepath.Join("sub", "file.txt") {
		t.Errorf("got %q", got)
	}
	if got := stripPath("repo-main", 1); got != "" {
		t.Errorf("wrapper-only should strip to empty, got %q", got)
	}
	// traversal is neutralized by Clean before stripping
	if got := stripPath("repo/../../etc/passwd", 1); reflect.DeepEqual(got, "../etc/passwd") {
		t.Errorf("path traversal leaked: %q", got)
	}
}
