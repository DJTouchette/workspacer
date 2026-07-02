// Package capspec is the small, dependency-free vocabulary shared between the
// bus (which enforces capability grants) and the plugin loader (which validates
// manifests and translates them into grants). Keeping it here avoids a bus↔plugin
// import cycle and keeps the list of filesystem-scoped capabilities in exactly
// one place, so enforcement and validation can never drift apart.
package capspec

// PathParam maps a capability method to the params field that carries the
// filesystem path it operates on. A method present here is "path-scoped": a
// plugin must declare path roots to be granted it, and the bus confines each
// call to those roots (see the bus's path-containment policy).
//
// This is the single source of truth for "which capabilities touch the
// filesystem". Add a method here the moment it grows a path argument, or it will
// be grantable to plugins without any path confinement.
var PathParam = map[string]string{
	"fs.read":        "path",
	"fs.write":       "path",
	"fs.listEntries": "path",
	"fs.listDir":     "path",
	"fs.watch":       "path",
	"fs.unwatch":     "path",
	"search.project": "cwd",
}

// IsPathScoped reports whether method operates on a filesystem path and, if so,
// the params field that carries it. Methods absent from PathParam carry no path
// and need no filesystem confinement.
func IsPathScoped(method string) (field string, ok bool) {
	field, ok = PathParam[method]
	return field, ok
}

// Grant is one capability a plugin token may call, with optional filesystem
// scoping. FSRoots, when set, restricts a path-scoped call to targets within one
// of the (canonical, absolute) roots; it is empty for non-path methods. Defined
// here — not in the bus — so the plugin loader can build grants without importing
// the bus, and the bus can accept them without importing the loader.
type Grant struct {
	Method  string
	FSRoots []string
}

// EventGrants is a plugin token's pub/sub + provider surface — the event side of
// the same "declare it in the manifest to be allowed it" model that [Grant]
// gives capability calls:
//
//   - Emits: event types the plugin may publish on the bus.
//   - Consumes: event types it may receive (delivery of anything else is dropped).
//   - Provides: capability method names it may register as a provider of.
//
// Patterns use the bus topic syntax — exact, "prefix.*", or "*" — matched by
// internal/event.Matches. Empty means none: a plugin that declared nothing can
// neither publish, receive, nor provide, matching the fail-closed stance of
// capability calls. Trusted connections (the host) bypass all of this.
type EventGrants struct {
	Emits    []string
	Consumes []string
	Provides []string
}
