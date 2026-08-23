---
title: claudemon wrap PTY Mirroring Wrapper
tags: [claudemon, rust, pty, portable-pty, websocket, wrapper]
related_paths:
  - "services/claudemon/src/wrapper/mod.rs"
  - "services/claudemon/src/wrapper/pty.rs"
owner: Damien Touchette
last_reviewed: 2026-07-11
---

# claudemon wrap PTY Mirroring Wrapper

## Overview
`claudemon wrap <command> [args...]` (entry `run_with_daemon` in `services/claudemon/src/wrapper/mod.rs`) is a standalone client process that spawns an arbitrary command in a local PTY via `portable-pty`, transparently forwards the invoking terminal's stdin/stdout to that child, and mirrors the PTY byte stream to the daemon over a `/wrapper/:id` WebSocket so the daemon can observe and remote-control the session (approve/deny, inject prompts, resize, signal). It is a client-side sibling to the daemon's own managed-provider PTYs, not the same code path.

## Key modules
- `services/claudemon/src/wrapper/mod.rs` — `run_with_daemon`: spawns the PTY, opens the daemon WS, wires the task topology, waits on child exit, sends `Exited`.
- `services/claudemon/src/wrapper/pty.rs` — `PtyHandle` and portable-pty bridging: `spawn`, `start_reader`, `write_bytes`/`write_bytes_blocking`, `resize`, `signal_child`, `has_exited`.
- `services/claudemon/src/protocol.rs` — `WrapperMessage` enum (`Register`, `Output`, `Exited`, `Input`, `Signal`, `Resize`) shared by wrapper client and daemon.
- `services/claudemon/src/daemon/wrapper_ws.rs` — daemon-side `/wrapper/:id` axum WS handler (`upgrade`/`handle`); expects `Register` as the first frame, then reads `Output`/`Exited`, writes `Input`/`Signal`/`Resize` via `WrapperHandle`.
- `services/claudemon/src/daemon/api.rs` (route registration `"/wrapper/:id"` at line 217) — mounts the WS route on the daemon's axum router.
- `services/claudemon/src/session/store.rs` — `kill_all_ptys` (session store's in-daemon PTY registry `self.ptys`), a different PTY population than the wrapper client's own `PtyHandle`.

## Failure modes
- **Daemon unreachable at connect**: `tokio_tungstenite::connect_async` failure is caught, logged (`"daemon unreachable; running detached"`), and the wrapper proceeds fully local — the `ws_rx` channel is drained by a no-op spawned task so any `ws_tx.send(...)` throughout the run never blocks or errors.
- **Mid-session WS send failure**: the `ws_rx → sink` writer task breaks its loop on send error (logs `"ws send failed"`) but does not tear down the PTY/stdin pumps — the local session keeps running silently unmirrored.
- **Malformed/unexpected frames**: non-`Text` WS frames and JSON decode errors from the daemon are logged and skipped (`continue`), not fatal.
- **First-frame contract**: daemon side requires `Register` as literally the first frame; anything else or a decode error causes `handle()` to return immediately, dropping the connection.
- **Exit sequencing**: child exit is awaited via blocking `Child::wait()` in `spawn_blocking`; after exit, raw mode is disabled, `Exited` is sent, then the code sleeps 50ms as a flush window before `pty_pump.abort()` / `stdin_pump.abort()` — a slow/backed-up WS send can still lose the `Exited` message if it exceeds that window.
- **PTY reader thread** (`start_reader`) uses a raw `std::thread::spawn` (not tokio) reading in 8KiB chunks; on read error or EOF it just breaks, closing the mpsc channel, which is what ends the `pty_pump` loop.

## Gotchas
- **All PTY bytes are base64 over the wire** — `WrapperMessage::Output`/`Input` carry `bytes: String` (base64), encoded/decoded via `base64::engine::general_purpose::STANDARD` (`B64`) on both send and receive; forgetting this on either side silently corrupts the stream.
- **Task topology in `run_with_daemon`**: (1) `ws_rx → sink` writer task, (2) daemon `stream → PTY` reader task (Input/Signal/Resize), (3) `PTY reader → stdout + daemon` pump (`pty_pump`, tokio task fed by `start_reader`'s std::thread via mpsc), (4) `stdin → PTY` pump (`spawn_blocking`, native OS thread). All four run concurrently; only `pty_pump` and `stdin_pump` are explicitly aborted at shutdown — the WS reader/writer tasks are left to die with the WS connection.
- **`Sigint` is split**: `Signal::Sigint` from the daemon writes a literal `\x03` (Ctrl-C byte) through the tty rather than sending a real OS signal, so it reaches the foreground process group like an interactive interrupt; `Sigterm`/`Sigkill` go through `pty::signal_child` (real `kill`/`SIGTERM` via `nix` on Unix, `Child::kill()` fallback elsewhere). `pty::signal_child` itself does NOT handle `Sigint` as a real signal for the same reason (comment explicitly calls this out).
- **`write_bytes` vs `write_bytes_blocking`**: `write_bytes` is async and internally does `spawn_blocking`; `write_bytes_blocking` is a plain synchronous call directly on the `Mutex<Box<dyn Write>>`. The stdin pump (already inside its own `spawn_blocking` thread) MUST use `write_bytes_blocking` — calling the async version via `block_on` there would re-enter the tokio runtime from a blocking thread (deadlock on current-thread runtimes, blocking-pool exhaustion on multi-thread).
- **Raw mode is conditional and must be paired**: `enable_raw_mode()` is only attempted when `stdin_is_tty` (checked via `IsTerminal`); `raw_enabled` is tracked and `disable_raw_mode()` is called only if it was actually enabled, right before sending `Exited` — skipping this on a non-interactive stdin (e.g. piped/CI invocation) is intentional, not a bug.
- **`extra_env` goes through `CommandBuilder`, never `std::env::set_var`**: `pty::spawn` first copies `std::env::vars()` into the `CommandBuilder`, then layers `extra_env` on top, entirely local to that builder — this avoids the data race that existed when concurrent spawns mutated process-global env.
- **Two distinct PTY populations**: the wrapper client's `PtyHandle` (this file) lives only in the `claudemon wrap` client process; the daemon's own managed-provider adapters own a separate PTY registry (`SessionStore.ptys`, see `services/claudemon/src/session/store.rs`), killed in bulk via `kill_all_ptys()` on daemon shutdown (`services/claudemon/src/daemon/mod.rs`). Do not conflate the two — `wrap`'s child is not tracked in `kill_all_ptys`.
- **`WrapperMessage` is bidirectional over one enum** — daemon→wrapper variants (`Input`, `Signal`, `Resize`) are silently ignored by the wrapper's write-loop's `_ => {}` arm when seen in the wrong direction, and vice versa; adding a new variant requires updating both `wrapper/mod.rs`'s match and the daemon's `wrapper_ws.rs` handling to keep them in sync.
