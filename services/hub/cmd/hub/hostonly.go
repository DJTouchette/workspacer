package main

// HOST-AUTHORITY ROUTES: the plugin-admin routes that make the HUB'S OWN HOST
// RUN CODE, and the one credential that may ask for that.
//
// The plugin install family — POST /plugins/install, /plugins/examples/install,
// /plugins/reload — is not "a mutating route". Install downloads a repository
// and runs the manifest's install argv; examples/install runs a bundled
// example's install step; reload loads plugin.json from a CALLER-NAMED
// directory, re-baselines its consented authority and starts its sidecar. Each
// of the three ends in a process running on the machine the hub runs on, as the
// user the hub runs as.
//
// guard() is not a strong enough gate for that, because guard() is
// srv.Authorized, and Authorized admits an OPERATOR-TIER SCOPED TOKEN as well
// as the host token. A remote worker node (deploy/fly/node) holds exactly such
// a token: it attaches as a capability provider, providing requires trust on
// the bus, and the operator tier is the tier that carries it. So the node's
// credential — a bearer string sitting in a Fly secret on a machine a thousand
// miles away, readable by anything that gets a shell there — could POST
// /plugins/install and have this host build and run whatever it named. The
// other shell-shaped capability that token reached, hub jobs, was already taken
// away from it operationally (the always-on hub is run with --jobs-file "", so
// jobs.* is never registered); this closes the half that was left.
//
// So these three routes ask for the HOST's own credential (bus.HostAuthorized).
// The distinction is real rather than a guess about who is calling: a node
// token is minted with `workspacer token create --scope operator` and lives in
// tokens.json, while the host token is a file on the hub that no node is ever
// handed. Nothing here depends on the caller's live bus connection, so a bare
// POST carrying a node token is refused the same as one from an attached node.
//
// WHAT THIS IS NOT. It is not the fix for "operator tier is promoted to host
// authority" — that is a bus-level tier redesign (a provider tier, so
// conn.mayProvide stops forcing trust), and it is being designed separately.
// This is the smallest true statement available today, and it costs the user's
// own local install nothing: the desktop, `workspacer plugin dev` and the CLI
// all present the host token (hubAuthHeaders → getHubToken → remote-token).
//
// THE GAP IT DOES NOT CLOSE, stated plainly: if an operator hands a node the
// HOST token instead of a scoped one, the node is the host and this gate sees
// nothing. deploy/fly/*/RUNBOOK.md mints a scoped token for exactly this
// reason and now says so.

import (
	"encoding/json"
	"log"
	"net/http"

	"github.com/djtouchette/workspacer-hub/internal/bus"
)

// hostOnlyRoute wraps a route that may only be reached by the host's own
// credential. `what` names the capability in the refusal ("plugin install"), so
// a 403 in a client log says which door was shut rather than "forbidden".
//
// Two refusals, deliberately distinct:
//
//   - 401 for a caller that would not pass guard() either — no token, a view /
//     triage tier, an unknown string. Unchanged from every other guarded route.
//   - 403 for a caller that IS authorized on this hub but does not hold host
//     authority — an operator-tier scoped token, i.e. a node. 403 is the honest
//     answer: the credential is real and the act is not available to it, and
//     answering 401 would send an operator hunting for a token problem.
//
// The log line names the refused credential (tier + label); the RESPONSE never
// does. Which token was presented is the hub operator's business, not the
// caller's — a caller that can enumerate labels can probe for one to steal.
func hostOnlyRoute(srv *bus.Server, what string, h http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if !srv.Authorized(r) {
			http.Error(w, "unauthorized", http.StatusUnauthorized)
			return
		}
		if !srv.HostAuthorized(r) {
			if si, ok := srv.ScopedIdentFor(r); ok {
				label := si.Label
				if label == "" {
					label = "(unlabelled)"
				}
				log.Printf("refused %s: the %s-tier scoped token %q does not hold host authority. %s runs code on this host, so it needs the host's own credential; a remote node's token is not one",
					what, si.Scope, label, what)
			} else {
				log.Printf("refused %s: caller does not hold host authority", what)
			}
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusForbidden)
			_ = json.NewEncoder(w).Encode(map[string]string{
				"error": what + " requires host authority: it runs code on the hub's own machine, so it is refused to every scoped bus token — including the operator tier, which is what a remote worker node carries. Run it from the machine that owns the hub.",
			})
			return
		}
		h(w, r)
	}
}
