package main

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/djtouchette/workspacer-hub/internal/capspec"
	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

// sensitiveManifest is a manifest whose every non-public field carries a
// distinctive marker string, so a leak is a substring search rather than a
// field-by-field opinion. The shape is the one actually observed in the hunt: a
// sidecar argv naming a private key file, an install command pointing at an
// internal registry, and an fs.read scope over ~/.ssh.
func sensitiveManifest() plugin.Manifest {
	return plugin.Manifest{
		ID:         "acme.secretscope",
		Name:       "Secret Scope",
		Version:    "1.3.0",
		APIVersion: plugin.APIVersion,
		Server: &plugin.ServerSpec{
			Command: "node",
			Args:    []string{"server.js", "--api-key-file", "/home/user/.ssh/MARKER_KEYPATH"},
			Port:    45999,
			Health:  "/MARKER_HEALTHPATH",
		},
		Install: []string{"npm", "install", "--registry", "https://MARKER_REGISTRY.internal/npm"},
		Source:  "https://MARKER_SOURCE.internal/plugins/secretscope.tgz",
		Dir:     "/home/user/.config/workspacer/plugins/MARKER_DIR",
		UI:      "MARKER_UIDIR",
		Capabilities: []plugin.Capability{
			{Method: "fs.read", Paths: []string{"/home/user/.ssh/MARKER_SCOPE"}},
		},
		Provides: []string{"acme.secretscope.MARKER_PROVIDES"},
		Emits:    []string{"acme.secretscope.MARKER_EMITS"},
		Consumes: []string{"MARKER_CONSUMES"},
		Settings: []plugin.SettingDef{
			{Key: "MARKER_SETTINGKEY", Label: "Endpoint", Type: plugin.SettingString, Default: "https://MARKER_SETTINGDEFAULT.internal/v2"},
		},
		// The public half.
		Panes:   []plugin.PaneContribution{{Type: "acme.tracker", Title: "Tracker", Path: "/ui"}},
		Widgets: []plugin.WidgetContribution{{ID: "lamp", Title: "Lamp"}},
		Hotkeys: []plugin.HotkeyContribution{{ID: "open", Default: "ctrl+shift+i", Command: "open-pane:acme.tracker"}},
	}
}

// markers are every value the event plane refuses a scoped tier. None may appear
// in an unauthenticated response.
var markers = []string{
	"MARKER_KEYPATH", "MARKER_HEALTHPATH", "MARKER_REGISTRY", "MARKER_SOURCE",
	"MARKER_DIR", "MARKER_UIDIR", "MARKER_SCOPE", "MARKER_PROVIDES",
	"MARKER_EMITS", "MARKER_CONSUMES", "MARKER_SETTINGKEY", "MARKER_SETTINGDEFAULT",
	"45999", "node",
}

// THE AGREEMENT, ON BYTES. plugin.loaded is TopicHostOnly: the event plane
// refuses this payload to every scoped tier and every plugin. The HTTP twin
// served it to callers with no credential at all. This drives the real handler
// and asserts the two planes now answer the same way about the same bytes.
func TestUnauthenticatedPluginListWithholdsWhatTheEventTwinRefuses(t *testing.T) {
	// Guard against the fixture rotting into a test of nothing: the twin has to
	// still be the host-only topic this is all about.
	if !capspec.EventTopicHostOnly("plugin.loaded") {
		t.Fatal("plugin.loaded is no longer host-only — this test's whole premise is that the event plane refuses these bytes")
	}
	h := manifestListHandler(
		func() []plugin.Manifest { return []plugin.Manifest{sensitiveManifest()} },
		func(r *http.Request) bool { return r.Header.Get("Authorization") == "Bearer HOSTTOKEN" },
	)

	req := httptest.NewRequest(http.MethodGet, "/plugins", nil)
	rec := httptest.NewRecorder()
	h(rec, req)
	if rec.Code != http.StatusOK {
		t.Fatalf("unauthenticated GET /plugins: code=%d", rec.Code)
	}
	body := rec.Body.String()
	for _, marker := range markers {
		if strings.Contains(body, marker) {
			t.Errorf("an unauthenticated GET /plugins disclosed %q. That value's event twin (plugin.loaded) is refused to every scoped tier; serving it here makes the three-tier registry decoration for these bytes.\nbody: %s", marker, body)
		}
	}
	// …and the public half is actually served, or the fix is a feature deletion
	// dressed as a boundary: a UI that cannot see pane/widget/hotkey
	// contributions cannot render the plugin at all.
	for _, want := range []string{"acme.secretscope", "Secret Scope", "1.3.0", "acme.tracker", "lamp", "ctrl+shift+i"} {
		if !strings.Contains(body, want) {
			t.Errorf("the public projection dropped %q — an unauthenticated client can no longer render this plugin's contributions", want)
		}
	}

	// The trusted host still gets everything: the desktop reads capabilities and
	// settings definitions out of this route to build the consent and settings UI.
	authed := httptest.NewRequest(http.MethodGet, "/plugins", nil)
	authed.Header.Set("Authorization", "Bearer HOSTTOKEN")
	rec = httptest.NewRecorder()
	h(rec, authed)
	full := rec.Body.String()
	for _, marker := range markers {
		if marker == "MARKER_DIR" {
			continue // Dir is json:"-" on the full manifest too
		}
		if !strings.Contains(full, marker) {
			t.Errorf("the authorized read lost %q — the host needs the whole manifest, and a projection that applies to everybody breaks the app instead of the leak", marker)
		}
	}
}

// An empty list must serialize as [] on both paths: `null` reads as "the hub is
// not answering", which is a distinction the desktop already paid for once.
func TestManifestListEncodesEmptyAsArray(t *testing.T) {
	h := manifestListHandler(func() []plugin.Manifest { return nil }, func(*http.Request) bool { return false })
	rec := httptest.NewRecorder()
	h(rec, httptest.NewRequest(http.MethodGet, "/plugins", nil))
	if got := strings.TrimSpace(rec.Body.String()); got != "[]" {
		t.Errorf("empty unauthenticated list = %q, want []", got)
	}
	h = manifestListHandler(func() []plugin.Manifest { return nil }, func(*http.Request) bool { return true })
	rec = httptest.NewRecorder()
	h(rec, httptest.NewRequest(http.MethodGet, "/plugins", nil))
	if got := strings.TrimSpace(rec.Body.String()); got != "[]" {
		t.Errorf("empty authorized list = %q, want []", got)
	}
}

// The projection is built by naming what it INCLUDES, and this is what makes
// that structural rather than stylistic: marshal the public view of a manifest
// whose every private field is a marker, and require the key set to be exactly
// the declared public one. A new Manifest field is therefore withheld by
// default — the direction a "redact these" projection gets wrong every time the
// struct grows.
func TestPublicManifestKeysAreAClosedSet(t *testing.T) {
	raw, err := json.Marshal(sensitiveManifest().Public())
	if err != nil {
		t.Fatal(err)
	}
	var got map[string]any
	if err := json.Unmarshal(raw, &got); err != nil {
		t.Fatal(err)
	}
	allowed := map[string]bool{
		"id": true, "name": true, "version": true, "apiVersion": true,
		"panes": true, "widgets": true, "hotkeys": true, "disabled": true,
	}
	for k := range got {
		if !allowed[k] {
			t.Errorf("the public plugin projection now emits %q. Add it to the allowed set here only after deciding it is safe for a caller with NO credential — the whole point of this struct is that growth is opt-in.", k)
		}
	}
	if len(got) < 6 {
		t.Fatalf("the projection emitted %d keys (%v) — it has collapsed, and a UI cannot render a plugin from it", len(got), got)
	}
}

// THE SETTINGS BLOCK. /plugins/ui/<id>/index.html is served to anyone — it is
// the plugin's own front-end and a webview URL cannot carry the host token —
// but the window.__WKS_SETTINGS__ block inside it is the same document
// /plugins/settings answers 401 for, and the same content
// plugin.settings.changed is TopicHostOnly for. Verified end to end here: the
// document is served either way; the values are only in the authorized one.
func TestPluginUIDocumentInjectsSettingsOnlyForAnEntitledCaller(t *testing.T) {
	if !capspec.EventTopicHostOnly("plugin.settings.changed") {
		t.Fatal("plugin.settings.changed is no longer host-only — the premise of this gate has moved")
	}
	uiDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(uiDir, "index.html"), []byte("<html><head></head><body>hi</body></html>"), 0o644); err != nil {
		t.Fatal(err)
	}
	values := map[string]any{
		"endpoint": "https://MARKER_PRODENDPOINT.internal/v2",
		"token":    "__WKS_SECRET__",
	}
	// The gate: the host token, or this plugin's own bus token as ?busToken=.
	authorizedFor := func(r *http.Request, id string) bool {
		return id == "acme.uiplug" &&
			(r.Header.Get("Authorization") == "Bearer HOSTTOKEN" ||
				r.URL.Query().Get("busToken") == "PLUGINTOKEN")
	}
	h := pluginUIHandler(
		stubUIResolver{id: "acme.uiplug", dir: uiDir},
		pluginSettingsForRequest(authorizedFor, func(string) (map[string]any, error) { return values, nil }),
	)

	doc := func(target string, mutate func(*http.Request)) string {
		req := httptest.NewRequest(http.MethodGet, target, nil)
		if mutate != nil {
			mutate(req)
		}
		rec := httptest.NewRecorder()
		h(rec, req)
		if rec.Code != http.StatusOK {
			t.Fatalf("%s: code=%d", target, rec.Code)
		}
		return rec.Body.String()
	}

	// Anonymous — including the rebinding spelling that was measured working.
	for _, mutate := range []func(*http.Request){
		nil,
		func(r *http.Request) { r.Host = "evil.example.com" },
		func(r *http.Request) { r.Header.Set("Authorization", "Bearer WRONGTOKEN") },
	} {
		body := doc("/plugins/ui/acme.uiplug/", mutate)
		if strings.Contains(body, "MARKER_PRODENDPOINT") || strings.Contains(body, "__WKS_SETTINGS__") {
			t.Errorf("an unentitled GET of the plugin document carried the settings block:\n%s", body)
		}
		// The document itself must still be served, or plugin UIs stop working.
		if !strings.Contains(body, "<script src=\"/plugins/sdk.js\">") || !strings.Contains(body, "__WKS_PLUGIN_ID__") {
			t.Errorf("the plugin document lost its SDK bootstrap for an anonymous caller:\n%s", body)
		}
	}

	// The host, and the plugin's own webview token, both get the values.
	host := doc("/plugins/ui/acme.uiplug/", func(r *http.Request) { r.Header.Set("Authorization", "Bearer HOSTTOKEN") })
	if !strings.Contains(host, "MARKER_PRODENDPOINT") {
		t.Errorf("the trusted host lost window.__WKS_SETTINGS__ — plugin panes would boot with no configuration:\n%s", host)
	}
	pane := doc("/plugins/ui/acme.uiplug/?busToken=PLUGINTOKEN", nil)
	if !strings.Contains(pane, "MARKER_PRODENDPOINT") {
		t.Errorf("a plugin webview carrying its own bus token lost its settings:\n%s", pane)
	}
}
