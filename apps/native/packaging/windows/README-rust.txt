Workspacer Native — Rust Preview (Windows x64)

This opt-in preview owns its Rust hub, engine, MCP and desktop services in process.
The Start menu shortcut runs wks-native.exe --local. Its state is isolated under
%LOCALAPPDATA%\Workspacer Native Rust Preview, outside the production Go/Node stack.
Use --rust-local-dir <directory> to choose another isolated state directory.
Bare wks-native.exe connects to an existing hub, as in the standard native build.

workspacer-rust.exe provides the standalone Rust server and client commands.
Run workspacer-rust.exe --help for explicit server directories and connection flags.
No Go service binaries, private Node runtime or desktop-host.cjs are included.
Node is used only by the build tooling; it is not required to start this backend.
Agent providers are optional external integrations, configured separately.

This unsigned preview has manual updates and a separate installer identity.
Installing or removing it does not replace the standard native or Electron app.
Uninstall preserves state and files that were not installed by this package.
This artifact does not assert complete migration parity or publish a nightly.
