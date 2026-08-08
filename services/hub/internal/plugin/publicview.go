package plugin

// THE HTTP/EVENT AGREEMENT FOR A MANIFEST.
//
// One plugin.Manifest has an EVENT twin (plugin.loaded) and an HTTP twin (GET
// /plugins). capspec classifies the event twin TopicHostOnly — refused outright
// to every scoped tier and to every plugin — for a payload it describes as
// "install argv and source, server command/args, and every declared filesystem
// path SCOPE, i.e. a map of what each sidecar may reach". The HTTP route served
// the same struct with no guard at all, to callers with no credential.
//
// Guarding the route outright would be the obvious answer and is the wrong one:
// a plugin webview and the /m PWA legitimately need to know which panes,
// widgets and hotkeys exist, and neither can carry the host token. So the two
// planes are made to agree on the BYTES instead of on the route: an
// unauthenticated GET gets this projection, and the projection is built by
// naming what goes IN rather than what is stripped out.
//
// That direction matters. A "redact these fields" projection re-opens itself
// every time Manifest grows a field — and Manifest is a manifest: it grows.
// PublicManifest is a separate struct, so a new Manifest field is absent from
// the public view until someone adds it here, and
// TestPublicManifestWithholdsEverythingTheEventTwinRefuses fails on any
// sensitive value that appears in the rendered bytes.

// PublicManifest is the projection of a Manifest served to an UNAUTHENTICATED
// caller: the plugin's identity and its UI contributions, and nothing about how
// it runs or what it may reach.
//
// Deliberately absent, each one the reason a field is absent:
//
//   - Server (command, args, port, health) — the argv the host executes and the
//     loopback port it listens on. The proven case declared
//     args:["--api-key-file","~/.ssh/id_ed25519"], so the argv alone named a
//     secret's location.
//   - Install — the argv that ran at install time, including private registry
//     URLs.
//   - Capabilities — every declared filesystem path scope: a map of what each
//     sidecar may reach, which is where to aim the next chain.
//   - Provides / Emits / Consumes — the plugin's bus surface.
//   - Settings — the setting DEFINITIONS. Values are redacted elsewhere; the
//     definitions still name internal endpoints and org/repo keys, and
//     plugin.settings.changed is host-only for exactly that content.
//   - Source — the install URL, frequently an internal host.
//   - Dir — an absolute host path (already json:"-", kept out by construction
//     here too rather than by that tag's continued existence).
//   - UI — whether and where static assets are served from on disk.
type PublicManifest struct {
	ID         string `json:"id"`
	Name       string `json:"name,omitempty"`
	Version    string `json:"version,omitempty"`
	APIVersion string `json:"apiVersion,omitempty"`

	// The contributions a UI has to know about to render this plugin at all.
	// Each carries only a type/id, a title, an icon and a URL path — no port, no
	// host path, no credential.
	Panes   []PaneContribution   `json:"panes,omitempty"`
	Widgets []WidgetContribution `json:"widgets,omitempty"`
	Hotkeys []HotkeyContribution `json:"hotkeys,omitempty"`

	// Disabled: a disabled plugin contributes nothing, and a client that does
	// not know cannot tell "absent" from "off".
	Disabled bool `json:"disabled,omitempty"`
}

// Public projects one manifest onto the unauthenticated view.
func (m Manifest) Public() PublicManifest {
	return PublicManifest{
		ID:         m.ID,
		Name:       m.Name,
		Version:    m.Version,
		APIVersion: m.APIVersion,
		Panes:      m.Panes,
		Widgets:    m.Widgets,
		Hotkeys:    m.Hotkeys,
		Disabled:   m.Disabled,
	}
}

// PublicManifests projects a list. It returns a non-nil empty slice so the JSON
// is `[]` rather than `null` — clients distinguish "no plugins" from "the hub
// is not answering", and that distinction already cost this codebase a bug (see
// the HUB_LIST_PLUGINS handler's comment).
func PublicManifests(ms []Manifest) []PublicManifest {
	out := make([]PublicManifest, 0, len(ms))
	for _, m := range ms {
		out = append(out, m.Public())
	}
	return out
}
