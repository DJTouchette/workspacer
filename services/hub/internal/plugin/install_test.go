package plugin

import (
	"archive/tar"
	"bytes"
	"compress/gzip"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strings"
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

	m, err := installFromTarball(dir, url, "fallback", InstallConsent{}, nil, nil, "")
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

// TestInstallFromFlatTarball covers a direct (non-GitHub) .tar.gz whose files
// sit at the archive root with no wrapping "<repo>-<ref>/" dir. The old code
// hard-stripped one leading path component for every archive, so every root
// entry of a flat tarball was discarded and the install failed with
// "no plugin.json found". locateManifestDir already handles root-or-one-level,
// so extraction must not blindly strip.
func TestInstallFromFlatTarball(t *testing.T) {
	data := rawTarGz(t, map[string]string{
		"plugin.json": `{"id":"flat.plugin","name":"Flat","apiVersion":"1"}`,
		"index.html":  "<html>flat</html>",
	})
	dir := t.TempDir()

	m, err := installFromTarball(dir, serveTarball(t, data), "flat", InstallConsent{}, nil, nil, "")
	if err != nil {
		t.Fatalf("flat tarball rejected: %v", err)
	}
	if m.ID != "flat.plugin" {
		t.Fatalf("manifest = %+v", m)
	}
	if _, err := os.Stat(filepath.Join(dir, "flat-plugin", "index.html")); err != nil {
		t.Errorf("index.html from a flat tarball not installed: %v", err)
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
	if _, err := installFromTarball(dir, mk("v1"), "r", InstallConsent{}, nil, nil, ""); err != nil {
		t.Fatal(err)
	}
	if _, err := installFromTarball(dir, mk("v2"), "r", InstallConsent{}, nil, nil, ""); err != nil {
		t.Fatal(err)
	}
	got, _ := os.ReadFile(filepath.Join(dir, "x-y", "index.html"))
	if string(got) != "v2" {
		t.Fatalf("reinstall did not overwrite: %q", got)
	}
}

// The build command is arbitrary code execution as the user, so it runs only
// with consent that names the exact argv.
var buildArgv = []string{"sh", "-c", "echo done > built.marker"}

func buildingPlugin(t *testing.T) string {
	t.Helper()
	return serveTarball(t, makeTarGz(t, "b-main", map[string]string{
		"plugin.json": `{"id":"b.uild","apiVersion":"1","install":["sh","-c","echo done > built.marker"]}`,
	}))
}

func TestInstallRunsBuildCommandWithConsent(t *testing.T) {
	dir := t.TempDir()
	consent := InstallConsent{Allow: true, Argv: buildArgv}
	if _, err := installFromTarball(dir, buildingPlugin(t), "b", consent, nil, nil, ""); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, "b-uild", "built.marker")); err != nil {
		t.Fatalf("install command did not run: %v", err)
	}
}

// Without consent the caller gets the argv to show the user — and, just as
// importantly, nothing has been written: no half-installed plugin sits there
// unbuilt while a dialog waits for an answer.
func TestInstallWithoutConsentAsksAndChangesNothing(t *testing.T) {
	dir := t.TempDir()
	_, err := installFromTarball(dir, buildingPlugin(t), "b", InstallConsent{}, nil, nil, "")
	var need *ConsentRequiredError
	if !errors.As(err, &need) {
		t.Fatalf("expected ConsentRequiredError, got %v", err)
	}
	if need.PluginID != "b.uild" || !slices.Equal(need.Argv, buildArgv) {
		t.Fatalf("consent request must carry what the user has to approve, got %+v", need)
	}
	if entries, _ := os.ReadDir(dir); len(entries) != 0 {
		t.Fatalf("refusing consent must leave the plugins dir untouched, found %v", entries)
	}
}

// The consent round-trip re-downloads, so a source that served a harmless
// command the first time could serve a different one the second. Consent is to
// the command the user read, not to whatever arrives next.
func TestInstallRefusesAnArgvThatChangedAfterConsent(t *testing.T) {
	dir := t.TempDir()
	consent := InstallConsent{Allow: true, Argv: []string{"npm", "run", "build"}}
	_, err := installFromTarball(dir, buildingPlugin(t), "b", consent, nil, nil, "")
	if err == nil {
		t.Fatal("expected the swapped install command to be refused")
	}
	var need *ConsentRequiredError
	if errors.As(err, &need) {
		t.Fatal("a swapped command must fail the install, not re-prompt")
	}
	if _, statErr := os.Stat(filepath.Join(dir, "b-uild", "built.marker")); statErr == nil {
		t.Fatal("the unapproved command ran")
	}
}

// A plugin with no install command never asks for anything.
func TestInstallWithNoBuildCommandNeedsNoConsent(t *testing.T) {
	data := makeTarGz(t, "q-main", map[string]string{
		"plugin.json": `{"id":"quiet","apiVersion":"1"}`,
	})
	dir := t.TempDir()
	if _, err := installFromTarball(dir, serveTarball(t, data), "q", InstallConsent{}, nil, nil, ""); err != nil {
		t.Fatalf("a plugin with no install command must install unprompted: %v", err)
	}
}

func TestInstallNoManifest(t *testing.T) {
	data := makeTarGz(t, "empty-main", map[string]string{"readme.md": "hi"})
	dir := t.TempDir()
	if _, err := installFromTarball(dir, serveTarball(t, data), "empty", InstallConsent{}, nil, nil, ""); err == nil {
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
	m, err := InstallFromDir(dir, src, nil, "")
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
	if _, err := InstallFromDir(dir, src, nil, ""); err != nil {
		t.Fatalf("re-add failed: %v", err)
	}
}

func TestExpandPlatformTokens(t *testing.T) {
	exe := ""
	if runtime.GOOS == "windows" {
		exe = ".exe"
	}
	got := expandPlatformTokens("./bin/${os}-${arch}/server${exe}")
	want := "./bin/" + runtime.GOOS + "-" + runtime.GOARCH + "/server" + exe
	if got != want {
		t.Errorf("expandPlatformTokens = %q, want %q", got, want)
	}
	// A command with no tokens is returned unchanged.
	if got := expandPlatformTokens("python3"); got != "python3" {
		t.Errorf("unexpected change: %q", got)
	}
	// Slice form expands each element.
	args := expandPlatformTokensAll([]string{"--bin", "x${exe}"})
	if args[1] != "x"+exe {
		t.Errorf("args expansion = %v", args)
	}
}

// rawTarGz builds a gzipped tar from files at the given paths verbatim (no wrap
// dir), for exercising the extraction bounds directly.
func rawTarGz(t *testing.T, files map[string]string) []byte {
	t.Helper()
	var buf bytes.Buffer
	gz := gzip.NewWriter(&buf)
	tw := tar.NewWriter(gz)
	for name, content := range files {
		if err := tw.WriteHeader(&tar.Header{Name: name, Mode: 0o644, Size: int64(len(content)), Typeflag: tar.TypeReg}); err != nil {
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

func TestExtractBoundsTotalBytes(t *testing.T) {
	data := rawTarGz(t, map[string]string{
		"a.txt": "hello world, this is more than the tiny total budget",
	})
	lim := extractLimits{maxBytes: 8, maxFiles: 100, maxFileSize: 1 << 20}
	err := extractTarGzLimited(bytes.NewReader(data), t.TempDir(), 0, lim)
	if err == nil || !strings.Contains(err.Error(), "total extraction limit") {
		t.Fatalf("expected total-limit error, got %v", err)
	}
}

func TestExtractBoundsSingleFile(t *testing.T) {
	data := rawTarGz(t, map[string]string{
		"big.bin": strings.Repeat("x", 64),
	})
	lim := extractLimits{maxBytes: 1 << 20, maxFiles: 100, maxFileSize: 16}
	err := extractTarGzLimited(bytes.NewReader(data), t.TempDir(), 0, lim)
	if err == nil || !strings.Contains(err.Error(), "per-file limit") {
		t.Fatalf("expected per-file-limit error, got %v", err)
	}
}

func TestExtractBoundsFileCount(t *testing.T) {
	data := rawTarGz(t, map[string]string{
		"a": "1", "b": "2", "c": "3", "d": "4", "e": "5",
	})
	lim := extractLimits{maxBytes: 1 << 20, maxFiles: 2, maxFileSize: 1 << 20}
	err := extractTarGzLimited(bytes.NewReader(data), t.TempDir(), 0, lim)
	if err == nil || !strings.Contains(err.Error(), "too many entries") {
		t.Fatalf("expected file-count error, got %v", err)
	}
}

func TestExtractWithinBoundsSucceeds(t *testing.T) {
	dir := t.TempDir()
	data := rawTarGz(t, map[string]string{
		"a.txt":     "small",
		"sub/b.txt": "also small",
	})
	lim := extractLimits{maxBytes: 1 << 20, maxFiles: 100, maxFileSize: 1 << 20}
	if err := extractTarGzLimited(bytes.NewReader(data), dir, 0, lim); err != nil {
		t.Fatalf("well-formed archive rejected: %v", err)
	}
	if got, _ := os.ReadFile(filepath.Join(dir, "a.txt")); string(got) != "small" {
		t.Errorf("a.txt = %q, want %q", got, "small")
	}
	if got, _ := os.ReadFile(filepath.Join(dir, "sub", "b.txt")); string(got) != "also small" {
		t.Errorf("sub/b.txt = %q, want %q", got, "also small")
	}
}

// TestInstallUnaffectedByBounds proves the production default bounds don't
// disturb a normal install (the existing happy path runs through the real
// extractTarGz → defaultExtractLimits).
func TestInstallUnaffectedByBounds(t *testing.T) {
	data := makeTarGz(t, "ok-main", map[string]string{
		"plugin.json": `{"id":"ok.plugin","name":"OK","apiVersion":"1"}`,
		"index.html":  "<html>ok</html>",
	})
	dir := t.TempDir()
	m, err := installFromTarball(dir, serveTarball(t, data), "ok", InstallConsent{}, nil, nil, "")
	if err != nil {
		t.Fatalf("normal install rejected by bounds: %v", err)
	}
	if m.ID != "ok.plugin" {
		t.Fatalf("manifest = %+v", m)
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

// A reinstall/update must (1) stop the running plugin first — on Windows a
// live sidecar's directory can't be moved or deleted — and (2) swap the whole
// directory rather than deleting in place, so a failure can never leave a
// half-gutted install (the "ui missing" pane).
func TestReinstallStopsPluginAndSwapsWholeDir(t *testing.T) {
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
	var stopped []string
	stop := func(id string) { stopped = append(stopped, id) }

	// Fresh install: nothing to stop.
	if _, err := InstallFromDir(dir, src, stop, ""); err != nil {
		t.Fatal(err)
	}
	if len(stopped) != 0 {
		t.Fatalf("stopForReplace called on fresh install: %v", stopped)
	}

	// A file only the OLD install has — a replace must swap, never merge.
	stray := filepath.Join(dir, "example-hello", "stale.txt")
	if err := os.WriteFile(stray, []byte("old"), 0o644); err != nil {
		t.Fatal(err)
	}

	if _, err := InstallFromDir(dir, src, stop, ""); err != nil {
		t.Fatalf("reinstall failed: %v", err)
	}
	if len(stopped) != 1 || stopped[0] != "example.hello" {
		t.Errorf("stopForReplace calls = %v, want [example.hello]", stopped)
	}
	if _, err := os.Stat(stray); err == nil {
		t.Error("replace merged into the old dir instead of swapping it")
	}
	if _, err := os.Stat(filepath.Join(dir, "example-hello", "ui", "index.html")); err != nil {
		t.Errorf("ui asset missing after replace: %v", err)
	}
	if stale, _ := filepath.Glob(filepath.Join(dir, ".trash-*")); len(stale) != 0 {
		t.Errorf("trash dirs left behind on the happy path: %v", stale)
	}
}

// Installer work dirs (.install-* temps from a crashed install, .trash-*
// moved-aside installs awaiting deletion) contain a plugin.json — LoadDir
// must never load them as plugins, or a failed update boots a duplicate.
func TestLoadDirIgnoresInstallerWorkDirs(t *testing.T) {
	dir := t.TempDir()
	manifest := `{"id":"example.real","name":"Real","apiVersion":"1","ui":"ui","panes":[{"type":"example.real","title":"Real"}]}`
	for _, sub := range []string{"real-plugin", ".trash-real-plugin-123", ".install-abc"} {
		if err := os.MkdirAll(filepath.Join(dir, sub), 0o755); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(dir, sub, "plugin.json"), []byte(manifest), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	ms, errs := LoadDir(dir)
	if len(errs) != 0 {
		t.Fatalf("LoadDir errors: %v", errs)
	}
	if len(ms) != 1 || ms[0].ID != "example.real" {
		t.Fatalf("LoadDir = %+v, want exactly the real plugin", ms)
	}
}

// A plugin's .bus-token is the credential the bus keys its whole capability
// grant on, and loadOrCreatePluginToken adopts an existing file verbatim. An
// archive that ships one therefore chooses its own authority — with no code
// execution, and for a webview-only plugin no install-consent prompt at all.
func TestInstallStripsLoaderOwnedSidecars(t *testing.T) {
	src := t.TempDir()
	writeFile(t, filepath.Join(src, "plugin.json"), `{"apiVersion":"1","id":"evil","name":"Evil","version":"1.0.0"}`)
	// Everything the loader owns, supplied by the "author".
	writeFile(t, filepath.Join(src, ".bus-token"), "attacker-known-token")
	writeFile(t, filepath.Join(src, ".settings.json"), `{"apiKey":"pre-seeded"}`)
	writeFile(t, filepath.Join(src, ".disabled"), "")
	writeFile(t, filepath.Join(src, ".install-source"), "github:someone/else")
	// Ordinary content must survive untouched.
	writeFile(t, filepath.Join(src, "index.html"), "<h1>hi</h1>")
	// A nested copy is inert content, not hub state — it stays.
	if err := os.MkdirAll(filepath.Join(src, "assets"), 0o755); err != nil {
		t.Fatal(err)
	}
	writeFile(t, filepath.Join(src, "assets", ".bus-token"), "not-a-credential-here")

	pluginsDir := t.TempDir()
	if _, err := InstallFromDir(pluginsDir, src, nil, ""); err != nil {
		t.Fatalf("install: %v", err)
	}

	dest := filepath.Join(pluginsDir, "evil")
	for _, name := range []string{".bus-token", ".settings.json", ".disabled", ".install-source"} {
		if _, err := os.Lstat(filepath.Join(dest, name)); !os.IsNotExist(err) {
			t.Errorf("%s survived the install — the archive chose the hub's own state", name)
		}
	}
	if _, err := os.Stat(filepath.Join(dest, "index.html")); err != nil {
		t.Errorf("real plugin content was stripped: %v", err)
	}
	if _, err := os.Stat(filepath.Join(dest, "assets", ".bus-token")); err != nil {
		t.Errorf("a nested dotfile is content, not hub state: %v", err)
	}
}

// A plugin's build step has to run on the same Node its sidecar will. The
// sidecar path already pinned `node` to the app's bundled runtime
// (Manager.sidecarNodeOverride) while the install path resolved `node` off the
// hub's PATH — so a plugin could fail to build on a machine with no system Node
// even though its sidecar would have started, or build against a different Node
// than the one that then loads the result.
func TestPinNodeRuntime(t *testing.T) {
	const runtimeBin = "/opt/workspacer/Workspacer"

	// No runtime configured (headless `workspacer serve`, tests): unchanged, and
	// a nil env so exec still inherits the hub's environment.
	bin, args, env := pinNodeRuntime([]string{"node", "build.js"}, "")
	if bin != "node" || !slices.Equal(args, []string{"build.js"}) || env != nil {
		t.Fatalf("without a runtime: bin=%q args=%v env=%v", bin, args, env)
	}

	// Configured: `node` runs on the bundled runtime, args preserved, and
	// ELECTRON_RUN_AS_NODE set — without it the binary boots as the desktop app.
	bin, args, env = pinNodeRuntime([]string{"node", "build.js", "--prod"}, runtimeBin)
	if bin != runtimeBin {
		t.Errorf("node not pinned to the bundled runtime: bin=%q", bin)
	}
	if !slices.Equal(args, []string{"build.js", "--prod"}) {
		t.Errorf("args not preserved: %v", args)
	}
	if !slices.Contains(env, "ELECTRON_RUN_AS_NODE=1") {
		t.Error("ELECTRON_RUN_AS_NODE missing — the runtime would launch as the app, not as Node")
	}
	if len(env) <= 1 {
		t.Error("env must extend the inherited environment, not replace it")
	}

	// Package managers are never rewritten: Electron ships no npm, so there is
	// nothing to re-point them at. Same for a plugin's own prebuilt binary.
	for _, argv := range [][]string{
		{"npm", "install"},
		{"npx", "tsc"},
		{"./bin/build.sh"},
		{"go", "build", "-o", "server"},
	} {
		if bin, _, env := pinNodeRuntime(argv, runtimeBin); bin != argv[0] || env != nil {
			t.Errorf("%v was rewritten: bin=%q env=%v", argv, bin, env)
		}
	}
}

// The rule for "is this the Node runtime" is shared with the sidecar path so the
// two can't drift into disagreeing. Package managers must not match it.
func TestIsNodeCommandAndToolchain(t *testing.T) {
	for _, cmd := range []string{"node", "node.exe"} {
		if !isNodeCommand(cmd) {
			t.Errorf("isNodeCommand(%q) = false", cmd)
		}
	}
	for _, cmd := range []string{"npm", "npx", "yarn", "pnpm", "./bin/server", "go", "sh"} {
		if isNodeCommand(cmd) {
			t.Errorf("isNodeCommand(%q) = true", cmd)
		}
	}
	for _, cmd := range []string{"npm", "npm.cmd", "npx", "yarn", "pnpm"} {
		if !isNodeToolchain(cmd) {
			t.Errorf("isNodeToolchain(%q) = false — a missing one needs the explanatory error", cmd)
		}
	}
	for _, cmd := range []string{"node", "sh", "go", "./bin/server"} {
		if isNodeToolchain(cmd) {
			t.Errorf("isNodeToolchain(%q) = true", cmd)
		}
	}
}

func writeFile(t *testing.T, path, body string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(body), 0o644); err != nil {
		t.Fatal(err)
	}
}
