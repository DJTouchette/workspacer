# Workspacer portable-pty patch

Source: crates.io portable-pty 0.8.1, upstream https://github.com/wez/wezterm.
The original MIT license and copyright are retained in LICENSE.md. Cargo.toml
and Cargo.toml.orig retain the published package metadata.

Only two upstream source files change:

- src/cmdbuilder.rs adds opt-in Windows creation flags, default zero in all
  constructors. The host-only field is excluded from serde to preserve the
  existing serialized command format.
- src/win/psuedocon.rs ORs those flags into the existing CreateProcessW flags.

Claudemon opts into CREATE_SUSPENDED so its per-child job owns the process before
its initial thread runs. Other callers and Unix behavior retain upstream defaults.
The host process is never assigned to a child job. This patch deliberately does
not change argument quoting, environment construction, pipe setup, or ConPTY
attribute handling.

The manifest declares the upstream legacy `cargo-clippy` feature as empty so
modern rustc check-cfg accepts the two existing Unix lint attributes. It is not
enabled by default and does not change runtime behavior.
