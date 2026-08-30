//! THE SERVED HALF OF THE claudemon HTTP CONTRACT.
//!
//! `contracts/claudemon-routes.json` is the machine-readable answer to "what
//! does claudemon actually serve". Everything that checks a CALLER against that
//! seam reads it — `services/hub/internal/capspec/claudemoncallers_test.go`
//! sweeps every caller in the repo against it, and `apps/tui`'s `mock_server`
//! answers 404 for a path it does not contain. So the fixture has to be the
//! router, not a description of it: a hand-maintained list drifts, and a drifted
//! list is exactly the disease. Commit `37320188` deleted the `/git` family and
//! `apps/tui` kept calling six of those routes for weeks with a green suite,
//! because nothing anywhere compared the two ends.
//!
//! This module is where the fixture is DERIVED. It reads the two router sources
//! at compile time (`include_str!`, so there is no runtime file lookup and no
//! way to point it at a stale copy), extracts every `.route()` registration, and
//! fails if the fixture disagrees. Regenerate with:
//!
//! ```text
//! make claudemon-routes
//! ```
//!
//! Test-only: nothing in the daemon consults this at run time. The router is the
//! router; this only reads it.

use std::fmt::Write as _;

/// The two router sources, verbatim, at compile time. `wrapper_ws.rs` and
/// `mcp_ask.rs` define handlers but register nothing outside a test module —
/// `capspec`'s own scan asserts that separately, from the Go side, over every
/// file in `daemon/`.
const API_RS: &str = include_str!("api.rs");
const HOOK_RS: &str = include_str!("hook.rs");

/// The fixture, as shipped.
const FIXTURE: &str = include_str!("../../../../contracts/claudemon-routes.json");

/// Ratchets on the SCAN. A scan that stops matching returns nothing, the
/// comparison below runs over two empty lists, and the test reports ok — the
/// failure mode every guard in this repo keeps re-learning. Raise when routes
/// are added.
const API_ROUTE_FLOOR: usize = 33;
const HOOK_ROUTE_FLOOR: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub(crate) struct Route {
    /// `"claudemon-api"` (:7891) or `"claudemon-hook"` (:7890). Two independent
    /// axum apps on two listeners; `/health` exists on both.
    pub server: String,
    /// The axum path AS REGISTERED, so `:id` is still a wildcard segment.
    pub pattern: String,
    /// The verb. Not decoration: `/mcp/ask/:session_id` is POST-only and axum
    /// answers a GET there with 405, so a GET caller is as dead as a caller of a
    /// route nobody registers.
    pub method: String,
}

/// Extract every `.route("<pattern>", <verb>(...))` from one router source.
///
/// Deliberately textual rather than clever: the alternative is asking axum for
/// its table, and `Router` exposes no such thing. Two shapes occur in these
/// files — the one-liner and the wrapped form where the path sits on its own
/// line — so the scan looks for `.route(` and then takes the next string
/// literal and the identifier that follows it, which covers both.
///
/// A `.route()` inside the file's own test module is a fixture, not an ingress,
/// so the source is truncated at `#[cfg(test)]` first.
pub(crate) fn scan_routes(src: &str, server: &str) -> Vec<Route> {
    let src = match src.find("#[cfg(test)]") {
        Some(i) => &src[..i],
        None => src,
    };
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(rel) = src[pos..].find(".route(") {
        let after = pos + rel + ".route(".len();
        pos = after;
        let Some(q) = src[after..].find('"') else {
            break;
        };
        let start = after + q + 1;
        let Some(e) = src[start..].find('"') else {
            break;
        };
        let end = start + e;
        let pattern = &src[start..end];

        // …then the verb: skip the comma and any whitespace/newlines, allow a
        // path-qualified spelling (`axum::routing::get`), and take the last
        // segment before the opening paren.
        let mut i = end + 1;
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b',' {
            i += 1;
        }
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        let vstart = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b':')
        {
            i += 1;
        }
        let verb = src[vstart..i]
            .rsplit("::")
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if verb.is_empty() {
            continue;
        }
        out.push(Route {
            server: server.to_string(),
            pattern: pattern.to_string(),
            method: verb,
        });
        pos = end + 1;
    }
    out
}

/// The whole table, sorted the way the fixture stores it.
pub(crate) fn scan_all() -> Vec<Route> {
    let mut all = scan_routes(API_RS, "claudemon-api");
    all.extend(scan_routes(HOOK_RS, "claudemon-hook"));
    all.sort();
    all
}

#[derive(serde::Deserialize)]
struct Fixture {
    routes: Vec<Route>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scan must be reading something, or every assertion below is a
    /// comparison between two empty lists.
    #[test]
    fn the_router_scan_is_not_reading_nothing() {
        let api = scan_routes(API_RS, "claudemon-api");
        let hook = scan_routes(HOOK_RS, "claudemon-hook");
        assert!(
            api.len() >= API_ROUTE_FLOOR,
            "scanned only {} routes out of api.rs (floor {API_ROUTE_FLOOR}) — the \
             axum .route() shape changed and this contract is derived from nothing",
            api.len()
        );
        assert!(
            hook.len() >= HOOK_ROUTE_FLOOR,
            "scanned only {} routes out of hook.rs (floor {HOOK_ROUTE_FLOOR})",
            hook.len()
        );
        // And a verb on every row: an unparsed verb would silently become the
        // empty string and match nothing on the caller side.
        for r in api.iter().chain(hook.iter()) {
            assert!(
                r.method == "GET" || r.method == "POST",
                "{} {} scanned verb {:?}, which is neither GET nor POST — the \
                 registration shape changed",
                r.server,
                r.pattern,
                r.method
            );
        }
    }

    /// THE PIN. `contracts/claudemon-routes.json` must be exactly what the two
    /// routers register — no more (a row nothing serves is a promise the daemon
    /// does not keep) and no less (a route outside the fixture is invisible to
    /// every caller guard that reads it, which is where `/git` hid).
    ///
    /// Set `UPDATE_CLAUDEMON_ROUTES=1` (or run `make claudemon-routes`) to
    /// rewrite the fixture from the routers instead of failing.
    #[test]
    fn the_route_fixture_is_the_router() {
        let scanned = scan_all();
        let fixture: Fixture =
            serde_json::from_str(FIXTURE).expect("contracts/claudemon-routes.json does not parse");

        if scanned == fixture.routes {
            return;
        }
        if std::env::var_os("UPDATE_CLAUDEMON_ROUTES").is_some() {
            rewrite_fixture(&scanned);
            return;
        }

        let mut msg = String::from(
            "contracts/claudemon-routes.json no longer describes the routers.\n\
             Run `make claudemon-routes` to regenerate it, then read the diff: a \
             route that VANISHED is a caller somewhere that now 404s.\n",
        );
        for r in &scanned {
            if !fixture.routes.contains(r) {
                let _ = writeln!(
                    msg,
                    "  + served, absent from the fixture: {} {} {}",
                    r.server, r.method, r.pattern
                );
            }
        }
        for r in &fixture.routes {
            if !scanned.contains(r) {
                let _ = writeln!(
                    msg,
                    "  - in the fixture, NOT served: {} {} {}",
                    r.server, r.method, r.pattern
                );
            }
        }
        panic!("{msg}");
    }

    /// Write the scanned table back into the fixture, keeping every prose field
    /// the file already carries. The prose is the part a human maintains; the
    /// `routes` array is the part that must never be hand-edited.
    fn rewrite_fixture(routes: &[Route]) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/claudemon-routes.json");
        let mut doc: serde_json::Value =
            serde_json::from_str(FIXTURE).expect("fixture does not parse");
        doc["routes"] = serde_json::to_value(routes).expect("routes serialize");
        let mut text = serde_json::to_string_pretty(&doc).expect("fixture serializes");
        text.push('\n');
        std::fs::write(&path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        eprintln!("rewrote {}", path.display());
    }

    /// The scanner has to survive the wrapped registration form, because six of
    /// the real routes use it — a scan that only matched the one-liner would
    /// silently drop `/sessions/spawn-managed`, `/providers/:provider/models`
    /// and `/sessions/:id/subagents/:agent_id/conversation`, and "absent from
    /// the fixture" reads identically to "deleted".
    #[test]
    fn the_scanner_reads_both_registration_shapes_and_ignores_test_modules() {
        let src = r#"
            Router::new()
                .route("/one", get(a))
                .route(
                    "/two/:id",
                    post(crate::daemon::spawn::handle_managed),
                )
                .route("/three", axum::routing::get(health))
            #[cfg(test)]
            mod tests { let _ = Router::new().route("/not-a-route", get(x)); }
        "#;
        let got = scan_routes(src, "srv");
        assert_eq!(
            got,
            vec![
                Route {
                    server: "srv".into(),
                    pattern: "/one".into(),
                    method: "GET".into()
                },
                Route {
                    server: "srv".into(),
                    pattern: "/two/:id".into(),
                    method: "POST".into()
                },
                Route {
                    server: "srv".into(),
                    pattern: "/three".into(),
                    method: "GET".into()
                },
            ]
        );
    }
}
