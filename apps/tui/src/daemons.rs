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

/// Ensure claudemon is up, spawning it if missing. Returns a guard that stops
/// whatever we started. No-ops when the URL isn't loopback or auto-spawn is off.
pub fn ensure(claudemon_url: &str, enabled: bool) -> Daemons {
    if !enabled || !is_loopback(claudemon_url) || port_open(CLAUDEMON_API_PORT) {
        return Daemons::none();
    }

    let mut daemons = Daemons::none();
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
    daemons
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
