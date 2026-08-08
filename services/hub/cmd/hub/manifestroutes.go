package main

import (
	"encoding/json"
	"net/http"

	"github.com/djtouchette/workspacer-hub/internal/plugin"
)

// manifestListHandler serves a list of plugin manifests at two different
// fidelities depending on who is asking.
//
// GET /plugins used to encode mgr.List() verbatim to anyone who asked, with no
// guard() — while the EVENT twin of those same bytes, plugin.loaded, is
// classified TopicHostOnly and refused to every scoped tier and every plugin.
// One plugin.Manifest, three dispositions in one process: refused as a call
// (plugins.* is in no scoped tier), refused as an event, served unauthenticated
// as HTTP. The event registry even conceded it in prose — "GET /plugins
// currently serves the same thing without a guard(); that route is a separate
// bug". This is that bug.
//
// The fix is not a guard on the route: a plugin webview and the /m PWA
// legitimately need the pane/widget/hotkey contributions and cannot carry the
// host token. It is a guard on the BYTES. An authorized caller (the desktop
// host, which already holds the token for /plugins/tokens) gets the full
// manifest; everyone else gets plugin.PublicManifest, which is built by naming
// what it includes, so a new Manifest field is withheld until someone adds it.
//
// authorized is srv.Authorized — the same predicate guard() uses, so "who counts
// as trusted" has exactly one definition on this plane.
// pluginSettingsForRequest is the gate on the settings block injected into a
// plugin's HTML at /plugins/ui/<id>/….
//
// The values are the merged non-secret setting values — endpoints, org/repo
// names, absolute paths. Secrets are already replaced with __WKS_SECRET__, so
// the leak was exactly the non-secret half, which is the half
// plugin.settings.changed's TopicHostOnly reason names word for word, and which
// GET /plugins/settings answers 401 for. They were being written into a document
// any caller could read, with any Host header, over the tailnet.
//
// The document itself stays public — it is the plugin's own front-end and a
// webview URL cannot carry the host token — so the gate is on the BLOCK:
// authorizedFor is srv.AuthorizedForPlugin, which admits the trusted host and
// the plugin's own bus/pane token (the ?busToken= the host injects into the
// webview URL) and nobody else. A refused caller gets the document with no
// settings seeded, which is the same state the SDK is in before the first
// settings event — a supported path, not a broken one.
func pluginSettingsForRequest(
	authorizedFor func(*http.Request, string) bool,
	get func(string) (map[string]any, error),
) func(*http.Request, string) map[string]any {
	return func(r *http.Request, id string) map[string]any {
		if !authorizedFor(r, id) {
			return nil
		}
		values, err := get(id)
		if err != nil {
			return nil
		}
		return values
	}
}

func manifestListHandler(list func() []plugin.Manifest, authorized func(*http.Request) bool) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		manifests := list()
		if manifests == nil {
			// `[]`, never `null`: a client reads null as a parse failure or a
			// missing list, and both callers here .map over the result.
			manifests = []plugin.Manifest{}
		}
		if authorized(r) {
			_ = json.NewEncoder(w).Encode(manifests)
			return
		}
		_ = json.NewEncoder(w).Encode(plugin.PublicManifests(manifests))
	}
}
