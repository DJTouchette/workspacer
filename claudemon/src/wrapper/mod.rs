//! PTY wrapper: `claudemon wrap claude ...`
//!
//! Plan (not yet implemented):
//!   1. Open a PTY via `portable-pty`.
//!   2. Spawn `argv[0] argv[1..]` attached to the PTY slave.
//!   3. Forward stdin → PTY master, PTY master → stdout, propagate SIGWINCH.
//!   4. Register the wrapper with the daemon (POST /wrappers) and serve a
//!      Unix-socket endpoint the daemon can call to inject bytes
//!      (approve/deny, new prompt, signal). The byte stream itself is also
//!      mirrored to the daemon so SSE subscribers see a live screen.

use anyhow::{bail, Result};

pub async fn run(argv: Vec<String>) -> Result<()> {
    if argv.is_empty() {
        bail!("usage: claudemon wrap <command> [args...]");
    }
    tracing::warn!(?argv, "claudemon wrap is not implemented yet");
    println!("claudemon wrap is a stub. Planned: spawn `{}` in a PTY", argv.join(" "));
    println!("and relay bytes + control to the daemon so clients can send input.");
    Ok(())
}
