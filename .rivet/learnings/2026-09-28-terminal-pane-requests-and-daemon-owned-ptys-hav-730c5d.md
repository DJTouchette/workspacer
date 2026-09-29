---
title: Terminal pane requests and daemon-owned PTYs have different ownership
date: 2026-09-28
suggested_doc: claudemon-pty-wrapper
related_paths:
  - services/hub-rs/src/services/terminals.rs
  - services/claudemon/src/daemon/embedded.rs
promoted: false
---

# Terminal pane requests and daemon-owned PTYs have different ownership

## Observation
terminals.open only emits facade.openTerminal; it must not independently start a child process. terminals.create delegates a login-shell allowlisted argv to claudemon, which owns and reaps that PTY. The bus byte stream uses an atomic ring snapshot plus live broadcast subscription. On lag, acquire a new pair before repainting with RIS; replaying a snapshot while retaining the old receiver can duplicate overlapping chunks. The Rust adapter records20-second leases per caller connection so one viewer detaching cannot terminate another viewers stream.
