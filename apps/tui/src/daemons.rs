//! Optional daemon bootstrap.
//!
//! `wks-tui` reads and controls agents through the **claudemon** daemon. When
//! run standalone there's no Electron app to start it, so unless told otherwise
//! we launch claudemon if it isn't already listening — mirroring how the
//! Electron main process spawns it:
//!
//!   claudemon serve --hook-port 7890 --api-port 7891
//!
//! An already-running claudemon (Electron, or one you started by hand) is left
//! untouched — we only spawn what's missing and only kill what we spawned.
//! Skipped entirely when the URL points at a non-loopback host.

use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const CLAUDEMON_API_PORT: u16 = 7891;
const CLAUDEMON_HOOK_PORT: u16 = 7890;

/// Guard over the daemon this process started. Dropping it stops claudemon, so
/// a daemon we launched doesn't outlive the TUI. A pre-existing one isn't here.
pub struct Daemons {
    children: Vec<(&'static str, Child)>,
}

impl Daemons {
    fn none() -> Self {
        Daemons { children: Vec::new() }
    }
}

impl Drop for Daemons {
    fn drop(&mut self) {
        for (name, child) in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
            eprintln!("[wks-tui] stopped {name}");
        }
    }
}

const HUB_BUS_PORT: u16 = 7895;

/// Ensure the daemons the TUI needs are up, spawning what's missing. Always
/// ensures claudemon; when `bus_url` is set (the TUI is a bus client), also
/// ensures the hub + its supervised brain. Returns a guard that stops only what
/// we started. No-ops when a URL isn't loopback or auto-spawn is off; an
/// already-running daemon (Electron, or one you started) is left untouched.
pub fn ensure(claudemon_url: &str, bus_url: Option<&str>, enabled: bool) -> Daemons {
    let mut daemons = Daemons::none();
    if !enabled {
        return daemons;
    }
    ensure_claudemon(&mut daemons, claudemon_url);
    if let Some(bus) = bus_url {
        ensure_hub(&mut daemons, bus, claudemon_url);
    }
    daemons
}

fn ensure_claudemon(daemons: &mut Daemons, claudemon_url: &str) {
    if !is_loopback(claudemon_url) || port_open(CLAUDEMON_API_PORT) {
        return; // remote, or already listening
    }
    match claudemon_bin() {
        Some(bin) => match spawn(&bin, &[
            "serve",
            "--hook-port",
            &CLAUDEMON_HOOK_PORT.to_string(),
            "--api-port",
            &CLAUDEMON_API_PORT.to_string(),
        ]) {
            Ok(child) => {
                eprintln!("[wks-tui] started claudemon ({})", bin.display());
                daemons.children.push(("claudemon", child));
                wait_for_port(CLAUDEMON_API_PORT, Duration::from_secs(5));
            }
            Err(e) => eprintln!("[wks-tui] could not start claudemon: {e}"),
        },
        None => eprintln!(
            "[wks-tui] claudemon not found — build it (cargo build --release in services/claudemon/) \
             or set WKS_CLAUDEMON_BIN. Staying in reconnect until it's reachable."
        ),
    }
}

/// Spawn the hub (with a full-scope brain) when the bus URL is loopback and
/// nothing is already listening there. The hub auto-detects the sibling brain
/// binary and bridges the given claudemon.
fn ensure_hub(daemons: &mut Daemons, bus_url: &str, claudemon_url: &str) {
    if !is_loopback(bus_url) {
        return; // someone else owns a remote hub
    }
    let (addr, port) = parse_bus_addr(bus_url);
    if port_open(port) {
        return; // hub already up — leave it
    }
    match hub_bin() {
        Some(bin) => {
            let events = format!("{claudemon_url}/events");
            let args: [&str; 8] = [
                "--addr", &addr,
                "--claudemon-events", &events,
                "--brain-scope", "full",
                "--claudemon", claudemon_url,
            ];
            match spawn(&bin, &args) {
                Ok(child) => {
                    eprintln!("[wks-tui] started hub + brain ({})", bin.display());
                    daemons.children.push(("hub", child));
                    wait_for_port(port, Duration::from_secs(5));
                }
                Err(e) => eprintln!("[wks-tui] could not start hub: {e}"),
            }
        }
        None => eprintln!(
            "[wks-tui] hub not found — build it (make build-hub) or set WKS_HUB_BIN. \
             --bus calls will fail until it's reachable."
        ),
    }
}

/// True when `url` is a loopback bus we'd manage but it isn't usable — the
/// signal to fall back to claudemon-direct. Two ways to be unusable:
///   * nothing is listening (e.g. the hub binary isn't built), or
///   * something is listening but it rejects our token with 401 (e.g. the
///     desktop owns the hub with a token we don't have / a stale one).
/// Without the 401 check a token mismatch would leave the bus client retrying
/// forever with an empty agent list — so we probe auth and fall back instead.
/// A remote bus is the user's responsibility, so it's never reported
/// unreachable here (we respect it and let the client reconnect).
pub fn loopback_bus_unreachable(url: &str, token: Option<&str>) -> bool {
    if !is_loopback(url) {
        return false;
    }
    let (addr, port) = parse_bus_addr(url);
    if !port_open(port) {
        return true; // nothing listening
    }
    bus_auth_rejected(&addr, bus_path(url), token) // listening but 401s our token
}

/// The path component of a `ws://host:port/path` bus URL, defaulting to `/bus`.
fn bus_path(url: &str) -> &str {
    let after = url.split("://").nth(1).unwrap_or(url);
    match after.find('/') {
        Some(i) => &after[i..],
        None => "/bus",
    }
}

/// Probe whether the hub rejects this token with 401. The hub checks the token
/// before the WebSocket upgrade, so a plain HTTP GET (no Upgrade headers) gets a
/// 401 when auth fails and some other status (an upgrade error) when it passes —
/// either way, a non-401 means our token is accepted. Any I/O hiccup is treated
/// as "not rejected" so a flaky probe never spuriously drops us off the bus.
fn bus_auth_rejected(addr: &str, path: &str, token: Option<&str>) -> bool {
    use std::io::{Read, Write};
    let Ok(sock) = addr.parse::<SocketAddr>() else { return false };
    let Ok(mut stream) = TcpStream::connect_timeout(&sock, Duration::from_millis(300)) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
    let target = match token {
        Some(t) if !t.is_empty() => format!("{path}?token={t}"),
        _ => path.to_string(),
    };
    let req = format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n");
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).unwrap_or(0);
    // Status line looks like "HTTP/1.1 401 Unauthorized".
    String::from_utf8_lossy(&buf[..n]).contains(" 401")
}

/// Extract `host:port` and the port from a `ws://host:port/path` bus URL,
/// defaulting the port to the hub's default when absent.
fn parse_bus_addr(url: &str) -> (String, u16) {
    let after = url.split("://").nth(1).unwrap_or(url);
    let authority = after.split('/').next().unwrap_or("");
    match authority.rsplit_once(':') {
        Some((host, port)) => {
            let p = port.parse().unwrap_or(HUB_BUS_PORT);
            (format!("{host}:{p}"), p)
        }
        None if !authority.is_empty() => (format!("{authority}:{HUB_BUS_PORT}"), HUB_BUS_PORT),
        None => (format!("127.0.0.1:{HUB_BUS_PORT}"), HUB_BUS_PORT),
    }
}

fn spawn(bin: &Path, args: &[&str]) -> std::io::Result<Child> {
    // Daemon output must not leak into the alternate-screen UI.
    Command::new(bin)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
}

/// Is anything accepting connections on this loopback port right now?
fn port_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

/// Block (briefly) until a port comes up, so the first request lands instead of
/// bouncing through reconnect backoff.
fn wait_for_port(port: u16, max: Duration) {
    let step = Duration::from_millis(100);
    let mut waited = Duration::ZERO;
    while waited < max {
        if port_open(port) {
            return;
        }
        std::thread::sleep(step);
        waited += step;
    }
}

/// Only manage a local daemon. A remote URL means someone else owns it.
fn is_loopback(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', ':'])
        .next()
        .unwrap_or("");
    matches!(host, "127.0.0.1" | "localhost" | "::1" | "")
}

// ── binary resolution ───────────────────────────────────────────────────────
//
// Env override wins; otherwise look in the in-repo build location relative to
// this crate. `CARGO_MANIFEST_DIR` is `<repo>/apps/tui`, so the repo root is two
// levels up — the common case when running via cargo or the in-tree binary.

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap_or_else(|| Path::new("."))
}

fn claudemon_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("WKS_CLAUDEMON_BIN") {
        return Some(PathBuf::from(p));
    }
    let name = if cfg!(windows) { "claudemon.exe" } else { "claudemon" };
    let target = repo_root().join("services").join("claudemon").join("target");
    for profile in ["release", "debug"] {
        let candidate = target.join(profile).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// The hub binary lives at `services/hub/hub` (where `make build-hub` puts it,
/// alongside the `brain` binary the hub auto-detects). Env override wins.
fn hub_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("WKS_HUB_BIN") {
        return Some(PathBuf::from(p));
    }
    let name = if cfg!(windows) { "hub.exe" } else { "hub" };
    let candidate = repo_root().join("services").join("hub").join(name);
    candidate.exists().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bus_addr_extracts_host_port() {
        assert_eq!(parse_bus_addr("ws://127.0.0.1:7895/bus"), ("127.0.0.1:7895".into(), 7895));
        assert_eq!(parse_bus_addr("ws://localhost:9000/bus"), ("localhost:9000".into(), 9000));
        // No port → hub default.
        assert_eq!(parse_bus_addr("ws://127.0.0.1/bus"), ("127.0.0.1:7895".into(), 7895));
    }

    #[test]
    fn loopback_detects_local_bus_urls() {
        assert!(is_loopback("ws://127.0.0.1:7895/bus"));
        assert!(is_loopback("ws://localhost:7895/bus"));
        assert!(!is_loopback("ws://example.com:7895/bus"));
    }

    #[test]
    fn unreachable_loopback_bus_triggers_fallback() {
        // Port 1 on loopback isn't listening → fall back to direct.
        assert!(loopback_bus_unreachable("ws://127.0.0.1:1/bus", None));
        // A remote bus is never reported unreachable (respected, not managed).
        assert!(!loopback_bus_unreachable("ws://example.com:1/bus", None));
        // A token doesn't change the closed-port verdict.
        assert!(loopback_bus_unreachable("ws://127.0.0.1:1/bus", Some("tok")));
    }

    #[test]
    fn bus_path_extracts_path_with_default() {
        assert_eq!(bus_path("ws://127.0.0.1:7895/bus"), "/bus");
        assert_eq!(bus_path("ws://127.0.0.1:7895/custom/path"), "/custom/path");
        assert_eq!(bus_path("ws://127.0.0.1:7895"), "/bus");
    }
}
