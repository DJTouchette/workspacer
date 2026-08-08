package plugin

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// THE `ui` ESCAPE. validateScope has refused a ".." segment in a capability
// path scope for rounds, with a comment naming the precise attack: the join
// that expands the scope Cleans the ".." away, so the declared path is not the
// granted one. The `ui` field, one field over, went through the identical join
// — filepath.Join(dir, ui) in UIDir — and was never checked at all.
//
// What that bought, measured on a scratch hub: ui="../../../.." made
// /plugins/ui/<id>/etc/passwd a 200, unauthenticated, with any Host header; and
// ui="." made the served root the plugin directory itself, so
// /plugins/ui/<id>/.bus-token returned the plugin's own bus bearer token, which
// then authenticated a /bus connection AS THAT PLUGIN — while the same
// anonymous caller was refused /bus outright.
//
// Neither the install consent dialog nor the grant pin covered it: pinOf() pins
// {Capabilities, Emits, Consumes, Provides} and not UI, so a widened `ui`
// survives a reload invisibly.
func TestManifestRefusesAUIPathThatEscapesThePluginDir(t *testing.T) {
	for _, ui := range []string{
		"..",
		"../..",
		"../../../..",
		"dist/../..",
		"..\\..",       // a manifest is portable JSON; both separators are written
		"dist\\..\\..", // the same, embedded
	} {
		m := Manifest{ID: "acme.esc", APIVersion: APIVersion, UI: ui}
		err := m.Validate()
		if err == nil {
			t.Errorf("ui=%q validated. filepath.Join Cleans that %q away, so the hub would serve an unauthenticated file server rooted outside the plugin tree.", ui, "..")
			continue
		}
		if !strings.Contains(err.Error(), "..") {
			t.Errorf("ui=%q was refused for the wrong reason: %v", ui, err)
		}
	}

	// "." and its spellings: the served root becomes the plugin dir, which holds
	// .bus-token and plugin.json.
	for _, ui := range []string{".", "./", "dist/..", "./."} {
		m := Manifest{ID: "acme.dotui", APIVersion: APIVersion, UI: ui}
		if err := m.Validate(); err == nil {
			t.Errorf("ui=%q validated, so /plugins/ui/<id>/.bus-token would serve the plugin's own bus credential to an anonymous caller", ui)
		}
	}

	// Absolute forms are refused rather than silently reinterpreted: today
	// filepath.Join treats them as relative, which contains them by accident.
	for _, ui := range []string{"/etc", "/", "C:/Windows", "c:\\Windows"} {
		m := Manifest{ID: "acme.absui", APIVersion: APIVersion, UI: ui}
		if err := m.Validate(); err == nil {
			t.Errorf("ui=%q validated — an absolute ui is a statement of intent this loader should refuse, not reinterpret", ui)
		}
	}

	// FLOOR: the legitimate shapes must still load, or the fix is a deletion of
	// webview-only plugins.
	for _, ui := range []string{"web", "dist", "ui/dist", "dist/assets", "a.b"} {
		m := Manifest{ID: "acme.ok", APIVersion: APIVersion, UI: ui}
		if err := m.Validate(); err != nil {
			t.Errorf("ui=%q was refused and it names a plain subdirectory: %v", ui, err)
		}
	}
	// And no `ui` at all is the common case (sidecar plugins).
	side := Manifest{ID: "acme.side", APIVersion: APIVersion}
	if err := side.Validate(); err != nil {
		t.Errorf("a manifest with no ui was refused: %v", err)
	}
}

// The loader is not the only door. UIDir is where the served root is COMPUTED,
// so it re-checks — the same defence-in-depth expandScope applies after
// validateScope. A manifest that reaches the manager some other way (a direct
// Add, a future loader) must not get a file server pointed at the host.
func TestUIDirRefusesAnEscapingUIEvenWhenTheManifestWasNeverValidated(t *testing.T) {
	reg := newFakeRegistrar()
	for _, ui := range []string{"..", "../../../..", "."} {
		m := loadedManager(t, reg, Manifest{ID: "acme.esc", Dir: "/plugins/acme", UI: ui})
		if dir, ok := m.UIDir("acme.esc"); ok {
			t.Errorf("UIDir served root %q for ui=%q — the escape was refused at load and granted at use", dir, ui)
		}
	}
	// The legitimate case still resolves.
	ok := loadedManager(t, reg, Manifest{ID: "acme.ok", Dir: "/plugins/acme", UI: "dist"})
	if dir, got := ok.UIDir("acme.ok"); !got || dir != filepath.FromSlash("/plugins/acme/dist") {
		t.Errorf("UIDir = (%q, %v), want (/plugins/acme/dist, true)", dir, got)
	}
}

// End to end through the loader: a plugin dropped into --plugins-dir at boot
// with an escaping ui must not load at all. This is the load path that needed no
// operator action in the proven chain — no install, no consent, no reload.
func TestLoadDirRejectsAPluginWhoseUIEscapes(t *testing.T) {
	root := t.TempDir()
	dir := filepath.Join(root, "esc")
	if err := os.MkdirAll(dir, 0o755); err != nil {
		t.Fatal(err)
	}
	manifest := `{"id":"acme.esc","apiVersion":"1","ui":"../../../.."}`
	if err := os.WriteFile(filepath.Join(dir, "plugin.json"), []byte(manifest), 0o644); err != nil {
		t.Fatal(err)
	}
	manifests, errs := LoadDir(root)
	if len(manifests) != 0 {
		t.Errorf("LoadDir accepted %d plugin(s) with an escaping ui: %+v", len(manifests), manifests)
	}
	if len(errs) == 0 {
		t.Error("LoadDir reported no error for a plugin whose ui escapes its directory")
	}
}

// TestUIDirRefusesASymlinkOutOfThePluginDirectory is the resolved half of the
// escape ValidateUIDir cannot see.
//
// ValidateUIDir judges the manifest STRING: "..", ".", absolute, drive letter.
// A plugin shipping `ui: "assets"` with assets -> / passes every one of those
// tests, and /plugins/ui/<id>/ is unauthenticated — so the round that added the
// string check closed the spelling and left the filesystem open. This drives the
// single place the served root is derived.
func TestUIDirRefusesASymlinkOutOfThePluginDirectory(t *testing.T) {
	dir := t.TempDir()
	outside := t.TempDir()
	if err := os.WriteFile(filepath.Join(outside, "loot"), []byte("SECRET"), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(outside, filepath.Join(dir, "assets")); err != nil {
		t.Skipf("symlinks unavailable on this host: %v", err)
	}

	m := NewManager(nil, nil)
	m.plugins["p"] = &loaded{manifest: Manifest{ID: "p", Dir: dir, UI: "assets"}}

	// The control: the string itself is impeccable, which is the whole point.
	if err := ValidateUIDir("assets"); err != nil {
		t.Fatalf("precondition: ValidateUIDir rejected %q on its own (%v) — this test would then prove nothing about the resolved check", "assets", err)
	}

	if got, ok := m.UIDir("p"); ok {
		t.Errorf("UIDir served %q for a `ui` that resolves to %q, outside the plugin directory %q. /plugins/ui/ is unauthenticated, so this is the filesystem behind an anonymous GET.", got, outside, dir)
	}

	// Floor: an ordinary subdirectory must still be served, or the fix is a
	// refusal rather than a confinement.
	if err := os.MkdirAll(filepath.Join(dir, "real"), 0o755); err != nil {
		t.Fatal(err)
	}
	m.plugins["q"] = &loaded{manifest: Manifest{ID: "q", Dir: dir, UI: "real"}}
	if _, ok := m.UIDir("q"); !ok {
		t.Error("UIDir refused an ordinary in-directory ui subdir — the containment check is denying everything, which passes the case above for the wrong reason")
	}
}
