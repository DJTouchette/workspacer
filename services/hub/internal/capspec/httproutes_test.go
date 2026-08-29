package capspec

import (
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

// THE COMPLETENESS GUARD FOR THE HTTP PLANE.
//
// Same shape as the two guards before it, and for the same reason: a table that
// validates itself cannot notice the route nobody wrote down. So this scans the
// REGISTRATION SITES — Go AddRoute/HandleFunc calls on the hub and the MCP
// facade, axum .route() calls in claudemon — and fails on a route the registry
// does not classify. A new route now fails until someone decides whether it is
// guarded, tiered, public, or loopback-confined, and writes down why.
//
// It also reads whether each hub site is wrapped in guard(), so the registry
// cannot claim a guard the code does not apply, or vice versa. That direction
// matters more than it looks: the registry is prose, and prose drifts silently.

// Ratchets on the SCAN, not on the registry. A scan that stops matching returns
// nothing, every loop runs zero times, and the guard reports ok — the failure
// mode this family of tests keeps re-learning. Raise them when routes are added.
const (
	hubRouteFloor       = 29 // 27 AddRoute + /bus + /health
	claudemonRouteFloor = 36
	mcpRouteFloor       = 3
)

var (
	// srv.AddRoute("/plugins", …) and mux.HandleFunc("/bus", …) / mux.Handle(…).
	goRouteRe = regexp.MustCompile(`(?:AddRoute|HandleFunc|Handle)\(\s*"([^"]+)"\s*,`)
	// axum: .route("/sessions", get(...)) — the path may sit on the next line.
	rustRouteRe = regexp.MustCompile(`\.route\(\s*\n?\s*"([^"]+)"`)
)

// routeSite is one scanned registration.
type routeSite struct {
	server  string
	pattern string
	file    string
	line    int
	// wrapperGuarded: the site wraps its handler in guard(…) / requireBearer(…).
	wrapperGuarded bool
	// hostOnlyWrapped: the site wraps its handler in hostOnly(…), the
	// host-authority gate — a strictly stronger check than guard(), applied to
	// the routes that make this host run code.
	hostOnlyWrapped bool
	// handlerGuarded: the site names a handler whose own body refuses with 401.
	// /bus is the case this exists for — its credential check is the handshake
	// inside handleBus, not a wrapper — and a route may not be recorded
	// RouteGuarded without one of the two.
	handlerGuarded bool
}

// TestEveryHTTPRouteIsClassified is the forcing function.
func TestEveryHTTPRouteIsClassified(t *testing.T) {
	sites := scanRouteSites(t)

	counts := map[string]int{}
	for _, s := range sites {
		counts[s.server]++
	}
	hubCount := counts["hub"]
	claudemonCount := counts["claudemon-api"] + counts["claudemon-hook"]
	if hubCount < hubRouteFloor {
		t.Fatalf("scanned only %d hub routes (floor %d) — the registration syntax changed and this guard is guarding nothing", hubCount, hubRouteFloor)
	}
	if claudemonCount < claudemonRouteFloor {
		t.Fatalf("scanned only %d claudemon routes (floor %d) — the axum .route() shape changed and every Rust route is now unexamined", claudemonCount, claudemonRouteFloor)
	}
	if counts["mcp"] < mcpRouteFloor {
		t.Fatalf("scanned only %d mcp facade routes (floor %d)", counts["mcp"], mcpRouteFloor)
	}

	seen := map[string]bool{}
	for _, s := range sites {
		key := s.server + " " + s.pattern
		seen[key] = true
		row, ok := HTTPRouteSpec(s.server, s.pattern)
		if !ok {
			t.Errorf("%s:%d registers %s %q and internal/capspec/httproutes.go says nothing about it. Every route on this plane must be classified before it ships: guarded, host-only, tiered-payload (naming the gate), public-by-decision, or loopback-confined — with the reason. The bytes a new route serves are exactly what the call plane and the event plane already decide for the same payload, and this is where those three answers are made to agree.",
				s.file, s.line, s.server, s.pattern)
			continue
		}
		// The registry may not claim a guard the code does not apply, nor
		// understate one it does. Two observable forms count as a credential
		// check: the WRAPPER form (guard(…) / requireBearer(…) at the site) and
		// the IN-HANDLER form (the named handler's own body refuses with 401).
		// A route with neither may not be recorded RouteGuarded — that is the
		// drift this file exists to catch, and it is the direction that leaks.
		if s.wrapperGuarded && !credentialRequired(row.Disposition) {
			t.Errorf("%s:%d wraps %s %q in a credential check, and the registry classifies it %q. A row that understates the code is how a guard gets deleted without anyone noticing the row still says 'public'.",
				s.file, s.line, s.server, s.pattern, row.Disposition)
		}
		// THE HOST-AUTHORITY PIN, both directions. hostOnly() is what keeps a
		// remote worker node's operator-tier token — which passes srv.Authorized,
		// and therefore passes guard() — from reaching the routes that run code on
		// this host. Downgrading such a site back to guard() while the row still
		// reads "host-only" is precisely the silent re-opening this plane exists
		// to refuse, and so is quietly relabelling the row while the code holds.
		if s.hostOnlyWrapped && row.Disposition != RouteHostOnly {
			t.Errorf("%s:%d wraps %s %q in hostOnly(), the host-authority gate, and the registry classifies it %q. Record the stronger gate: a row that reads %q says an operator-tier scoped token may call it, and the code says it may not.",
				s.file, s.line, s.server, s.pattern, row.Disposition, row.Disposition)
		}
		if !s.hostOnlyWrapped && row.Disposition == RouteHostOnly {
			t.Errorf("%s:%d registers %s %q with no hostOnly() wrapper, and the registry records it %q. That row is the claim that a node's operator-tier token is refused here; without the wrapper it is refused nothing, and this route runs code on the hub's own machine.",
				s.file, s.line, s.server, s.pattern, RouteHostOnly)
		}
		// The IN-HANDLER form understates in exactly the same direction. /bus is
		// the whole reason handlerGuarded exists — its credential check is the
		// handshake inside handleBus, not a wrapper — and it is the route that
		// classifies every presented token into its tier. Relabelling it away from
		// RouteGuarded (to public/tiered) would read as "no credential needed"
		// while the code still refuses with 401, and then the deletion of that 401
		// would ALSO pass, because line 94 won't fire once the row already reads
		// non-guarded. Pinning the handler form closes that two-step.
		if s.handlerGuarded && !credentialRequired(row.Disposition) {
			t.Errorf("%s:%d registers %s %q with a handler that refuses unauthenticated callers (a 401 in its own body), and the registry classifies it %q. A route the code guards in-handler must stay RouteGuarded — understating it is how the guard, and then the 401 itself, get removed with the row still reading non-guarded.",
				s.file, s.line, s.server, s.pattern, row.Disposition)
		}
		if !s.wrapperGuarded && !s.handlerGuarded && credentialRequired(row.Disposition) {
			t.Errorf("%s:%d registers %s %q with no credential check the scanner can see — neither a guard()/requireBearer() wrapper nor a 401 in the handler it names — and the registry records it as %q. The classification says a credential is required and the code hands the bytes to anybody.",
				s.file, s.line, s.server, s.pattern, RouteGuarded)
		}
	}

	// The other direction: a row nothing registers is a classification of
	// nothing, and would hide the rename that silenced a real one.
	for _, row := range HTTPRoutes() {
		if !seen[row.Server+" "+row.Pattern] {
			t.Errorf("the registry classifies %s %q and no registration site produces it. Either the route was renamed (and its new name is now unclassified) or the row is dead — a row that cannot fire is indistinguishable from one that works.", row.Server, row.Pattern)
		}
	}
}

// TestUnguardedRoutesAgreeWithTheirBusTwin is THE CROSS-PLANE PIN, and the
// thing this round existed to build: bytes the bus refuses a tier must not come
// out of an unauthenticated GET.
//
// The event registry classifies plugin.loaded and plugin.settings.changed
// TopicHostOnly — refused to every scoped tier and to every plugin. GET /plugins
// served the identical Manifest to a caller with no credential at all, and
// /plugins/ui/<id>/ inlined the identical setting values into an anonymous HTML
// document. Nothing in the repo compared the two planes, so both were "closed"
// and one of them was open.
func TestUnguardedRoutesAgreeWithTheirBusTwin(t *testing.T) {
	checked := 0
	for _, row := range HTTPRoutes() {
		if !HTTPRouteServesHostOnlyPayload(row) {
			continue
		}
		checked++
		switch row.Disposition {
		case RouteGuarded, RouteHostOnly:
			// The strictest answer: the route requires the same trust the topic
			// does (host-only requires strictly more).
		case RouteTieredPayload:
			if row.Gate == "" {
				t.Errorf("%s %q serves the payload of %q, which the event registry refuses every scoped tier, and it is classified %q with NO gate named. 'Some callers get less' is a claim about a mechanism; name the mechanism.",
					row.Server, row.Pattern, row.Twin, row.Disposition)
			}
		default:
			t.Errorf("%s %q is classified %q and carries the payload of %q, which the event plane classifies TopicHostOnly — refused to every scoped tier and every plugin. One payload cannot be host-only as an event and public as HTTP: either guard the route, or split the payload with a named gate and say what the unauthenticated half is.",
				row.Server, row.Pattern, row.Disposition, row.Twin)
		}
	}
	if checked < 3 {
		t.Fatalf("only %d routes were checked against a host-only twin — the twin links have been dropped from the registry and this pin is comparing nothing", checked)
	}
}

// TestHTTPRouteRegistryIsWellFormed holds each row to the shape that makes it
// mean something.
func TestHTTPRouteRegistryIsWellFormed(t *testing.T) {
	rows := HTTPRoutes()
	if len(rows) < hubRouteFloor {
		t.Fatalf("the registry holds %d rows — it shrank", len(rows))
	}
	hubSrc := string(mustReadRepoFile(t, "services", "hub", "cmd", "hub", "main.go")) +
		string(mustReadRepoFile(t, "services", "hub", "cmd", "hub", "manifestroutes.go")) +
		string(mustReadRepoFile(t, "services", "hub", "internal", "bus", "bus.go"))

	seen := map[string]bool{}
	byDisposition := map[RouteDisposition]int{}
	for _, r := range rows {
		key := r.Server + " " + r.Pattern
		if seen[key] {
			t.Errorf("two rows classify %s — a failure would name the wrong one", key)
		}
		seen[key] = true
		byDisposition[r.Disposition]++

		// A guarded row states what it protects in a phrase; every other
		// disposition IS a decision, and the reason is its whole content.
		min := 60
		if r.Disposition == RouteGuarded {
			min = 40
		}
		// A host-only row does not get the guarded row's shorter minimum: it
		// claims a gate above the token, and the reason has to say what the act
		// is and why a credential that passes every other guarded route is
		// nonetheless refused here.
		if len(strings.TrimSpace(r.Reason)) < min {
			t.Errorf("%s has no real reason (%q). For a guarded route, what it does and why that needs the token; for every other disposition, what an uncredentialed caller receives and why that is acceptable.", key, r.Reason)
		}
		switch r.Disposition {
		case RouteGuarded, RouteHostOnly, RoutePublic, RouteLoopbackConfined:
			if r.Gate != "" {
				t.Errorf("%s is %q and also names gate %q — a gate splits a TIERED payload; on any other disposition it is a mechanism nothing consults", key, r.Disposition, r.Gate)
			}
		case RouteTieredPayload:
			if r.Gate == "" {
				t.Errorf("%s is tiered-payload and names no gate. The tiering is the claim; the gate is the evidence", key)
			} else if !strings.Contains(hubSrc, r.Gate) {
				t.Errorf("%s names gate %q and no such identifier exists in the hub's route sources — a gate nobody calls is prose", key, r.Gate)
			}
		default:
			t.Errorf("%s has disposition %q, which is not one of the five", key, r.Disposition)
		}

		// A twin has to resolve, or the agreement test above compares nothing.
		switch r.TwinKind {
		case TwinNone:
			if r.Twin != "" {
				t.Errorf("%s names twin %q with no TwinKind — the link cannot be resolved in any registry", key, r.Twin)
			}
		case TwinEvent:
			if _, ok := EventTopicSpec(r.Twin); !ok {
				t.Errorf("%s names event twin %q, which the event-topic registry does not classify. A twin link is only worth something if the other end has an answer", key, r.Twin)
			}
		case TwinMethod:
			if r.Twin == "*" {
				continue // the bus routes themselves: every method, by definition
			}
			if MissingClassification(r.Twin) {
				t.Errorf("%s names bus twin %q, which capspec says nothing about — a twin that cannot be reasoned about is not a comparison", key, r.Twin)
			}
		default:
			t.Errorf("%s has TwinKind %q, which is not one of the three", key, r.TwinKind)
		}
	}
	// All five dispositions must stay in use. If tiered-payload ever empties,
	// the plane has collapsed back to "guarded or open", under which /plugins is
	// unclassifiable and therefore open — which is exactly where it started; and
	// if host-only empties, the plugin install family has been handed back to
	// every operator-tier token, i.e. to every remote node.
	for _, d := range []RouteDisposition{RouteGuarded, RouteHostOnly, RouteTieredPayload, RoutePublic, RouteLoopbackConfined} {
		if byDisposition[d] == 0 {
			t.Errorf("no row uses disposition %q — the registry has collapsed to a vocabulary that cannot express every route", d)
		}
	}
}

// TestLoopbackConfinedRoutesActuallyHaveTheirConfinement makes the
// loopback-confined disposition a CHECKED claim rather than a comforting word.
//
// Every claudemon route is unauthenticated. The registry says that is acceptable
// because the OS is the boundary: a foreign Host is refused (host_guard) and a
// cross-site Origin is refused (origin_guard), on both routers. Those two layers
// are the entire basis of ~35 rows, so their presence is asserted here rather
// than assumed — deleting either one silently promotes every one of those rows
// from "loopback-confined" to "reachable from any web page the user visits".
func TestLoopbackConfinedRoutesActuallyHaveTheirConfinement(t *testing.T) {
	routers := map[string][]string{
		"claudemon-api":  {"services", "claudemon", "src", "daemon", "api.rs"},
		"claudemon-hook": {"services", "claudemon", "src", "daemon", "hook.rs"},
	}
	for server, path := range routers {
		src := string(mustReadRepoFile(t, path...))
		// Truncate at the test module: a layer applied only in tests confines
		// nothing in production.
		if i := strings.Index(src, "#[cfg(test)]"); i > 0 {
			src = src[:i]
		}
		for _, layer := range []string{"host_guard", "origin_guard"} {
			// APPLIED, not merely defined. The first version of this check was a
			// substring search for the name, and removing the .layer() call from
			// the router left the `pub(crate) fn origin_guard` definition in the
			// same file — so the guard passed while the router was open. A
			// mutation test caught it, which is the entire argument for running
			// one against every guard this round added.
			applied := regexp.MustCompile(`\.layer\((?s).{0,160}?` + regexp.QuoteMeta(layer)).MatchString(src)
			if !applied {
				t.Errorf("%s no longer APPLIES %s as a router layer (the function may still exist; nothing calls it). Every %q row on this router rests on that layer: without it a DNS-rebound or cross-site page reaches handlers that spawn processes, drive PTYs and forge host-owned events.",
					server, layer, RouteLoopbackConfined)
			}
		}
	}
	// And at least one route must actually be classified this way, or the loop
	// above is checking layers nothing depends on.
	n := 0
	for _, r := range HTTPRoutes() {
		if r.Disposition == RouteLoopbackConfined {
			n++
		}
	}
	if n < claudemonRouteFloor {
		t.Fatalf("only %d loopback-confined rows (want >= %d) — the claudemon plane has stopped being classified", n, claudemonRouteFloor)
	}
}

// credentialRequired reports whether a disposition claims the route needs a
// credential at all. Both members of the family qualify: RouteGuarded (the bus
// token) and RouteHostOnly (the host's own token, refusing the operator-tier
// scoped tokens guard() admits). The drift checks ask this rather than comparing
// to RouteGuarded, so promoting a route to the stronger gate does not read as
// "the registry claims a guard the code does not apply".
func credentialRequired(d RouteDisposition) bool {
	return d == RouteGuarded || d == RouteHostOnly
}

// ---- scanning ---------------------------------------------------------

func scanRouteSites(t *testing.T) []routeSite {
	t.Helper()
	var out []routeSite

	// The hub: main.go's AddRoute sites plus the two the bus registers itself.
	out = append(out, scanGoRoutes(t, "hub", filepath.Join("services", "hub", "cmd", "hub", "main.go"))...)
	out = append(out, scanGoRoutes(t, "hub", filepath.Join("services", "hub", "internal", "bus", "bus.go"))...)
	// The MCP facade.
	out = append(out, scanGoRoutes(t, "mcp", filepath.Join("services", "hub", "cmd", "mcp", "main.go"))...)
	// claudemon's two routers.
	out = append(out, scanRustRoutes(t, "claudemon-api", filepath.Join("services", "claudemon", "src", "daemon", "api.rs"))...)
	out = append(out, scanRustRoutes(t, "claudemon-hook", filepath.Join("services", "claudemon", "src", "daemon", "hook.rs"))...)

	// A route registered in a Rust file NEITHER router owns would be invisible
	// above, so account for those too rather than trusting the file list.
	for _, name := range []string{"wrapper_ws.rs", "mcp_ask.rs", "oneshot.rs", "heartbeat.rs", "spawn.rs", "init.rs", "mod.rs"} {
		extra := scanRustRoutes(t, "claudemon-api", filepath.Join("services", "claudemon", "src", "daemon", name))
		for _, s := range extra {
			// api.rs already registers these; a NEW path here is a router nobody
			// classified.
			if _, ok := HTTPRouteSpec("claudemon-api", s.pattern); !ok {
				t.Errorf("%s:%d registers route %q outside the two known routers, and nothing classifies it", s.file, s.line, s.pattern)
			}
		}
	}
	return out
}

func scanGoRoutes(t *testing.T, server, rel string) []routeSite {
	t.Helper()
	body := string(mustReadRepoFile(t, splitPath(rel)...))
	lines := strings.Split(body, "\n")
	var out []routeSite
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.HasPrefix(trimmed, "//") {
			continue
		}
		m := goRouteRe.FindStringSubmatch(line)
		if m == nil {
			continue
		}
		site := routeSite{server: server, pattern: m[1], file: rel, line: i + 1}
		site.wrapperGuarded = strings.Contains(line, "guard(") || strings.Contains(line, "requireBearer(") || strings.Contains(line, "requireScope(") || strings.Contains(line, "hostOnly(")
		// hostOnly() is guard() plus a host-authority check, and the registry has
		// to record the STRONGER of the two or the row understates the code.
		site.hostOnlyWrapped = strings.Contains(line, "hostOnly(")
		site.handlerGuarded = handlerRefuses401(body, line)
		out = append(out, site)
	}
	if len(out) == 0 {
		t.Fatalf("no routes scanned from %s — the registration shape changed and this scan reports PASS while reading nothing", rel)
	}
	return out
}

func scanRustRoutes(t *testing.T, server, rel string) []routeSite {
	t.Helper()
	body := string(mustReadRepoFile(t, splitPath(rel)...))
	// A .route() inside the file's test module is a fixture, not an ingress.
	if i := strings.Index(body, "#[cfg(test)]"); i > 0 {
		body = body[:i]
	}
	var out []routeSite
	for _, loc := range rustRouteRe.FindAllStringSubmatchIndex(body, -1) {
		pattern := body[loc[2]:loc[3]]
		line := 1 + strings.Count(body[:loc[0]], "\n")
		out = append(out, routeSite{server: server, pattern: pattern, file: rel, line: line})
	}
	return out
}

// handlerNameRe pulls the handler identifier out of a registration line:
// mux.HandleFunc("/bus", s.handleBus) → handleBus.
var handlerNameRe = regexp.MustCompile(`,\s*(?:[A-Za-z_]\w*\.)?([A-Za-z_]\w*)\s*\)`)

// handlerRefuses401 reports whether the handler a registration line NAMES
// refuses unauthenticated callers in its own body. It is deliberately narrow:
// it resolves only the `func (x *T) name(` / `func name(` form in the same file,
// and only counts an explicit 401. An inline closure returns false — a closure
// that tiers its payload (like /app/) is not the same claim as a route that
// refuses outright, and conflating them is how "split auth" becomes "guarded".
func handlerRefuses401(fileBody, line string) bool {
	m := handlerNameRe.FindStringSubmatch(line)
	if m == nil {
		return false
	}
	name := m[1]
	idx := regexp.MustCompile(`func\s+(?:\([^)]*\)\s*)?` + regexp.QuoteMeta(name) + `\(`).FindStringIndex(fileBody)
	if idx == nil {
		return false
	}
	body := fileBody[idx[0]:]
	if end := strings.Index(body, "\n}\n"); end > 0 {
		body = body[:end]
	}
	return strings.Contains(body, "StatusUnauthorized")
}

func splitPath(rel string) []string {
	return strings.Split(filepath.ToSlash(rel), "/")
}
