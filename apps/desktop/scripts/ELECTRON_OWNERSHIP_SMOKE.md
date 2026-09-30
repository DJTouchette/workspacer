# Packaged Electron ownership smoke

Linux only, with Xvfb and an already-built unpacked Electron package:

```sh
xvfb-run -a make test-electron-ownership \
  WKS_ELECTRON_EXECUTABLE=/absolute/release/linux-unpacked/workspacer \
  WKS_ELECTRON_SMOKE_REPORT=/absolute/release/electron-ownership-smoke.json
```

Missing inputs, packaged resources or display/runtime dependencies fail. The
command never builds a replacement artifact and never silently skips. It uses
existing Playwright, Node and WebSocket tooling.

The two cases launch the real packaged main entrypoint with isolated config,
home and user data. Existing WORKSPACER_PORT_OFFSET selects a checked four-port
set; polling/startup provider checks and automatic updates are disabled in that
private configuration. No model is launched.

The owned case requires the actual desktop-provided config.get marker over the
Rust bus and the modern MCP catalog including its cache metadata, unique names and captured100-tool operator floor. Exact MCP health must name the Rust implementation, live hub connection, initialized catalog and expected listen/hub addresses.
It explicitly invokes app.quit, which takes the production before-quit path,
then requires the actual Electron process exit code0 with no signal and all four
listeners closed. It does not count a successful Playwright close call as proof:
the installed Playwright implementation has forced cleanup fallback behavior.

The adopted case starts the packaged Rust control plane externally first.
Electron must leave its bus/MCP listeners alive after normal quit while retiring
its own claudemon listeners. Only afterward does the harness close the external
fixture's parent pipe and require its own successful exit and closed listeners.
Failure cleanup is never converted to passing evidence. Reports include the CI
revision when available; diagnostics contain only private-fixture state and
redact its token.

This checks process ownership, not visual GUI parity, installed OS integration,
provider authentication or model behavior. Existing standalone archive and
Native Rust Preview installer smokes retain their separate scopes.

Local implementation checkpoint: helper5 tests pass using TMPDIR=/dev/shm and
Node syntax/whitespace checks pass. Default /tmp helper execution hit ENOSPC
(3pass/1environmental failure); it is not counted as a pass. No packaged artifact,
standalone claudemon or Xvfb was available locally, so real ownership execution
awaits the authorized release preview. Do not mark an Electron/package gate from
helper tests alone.

The Linux release leg runs helpers independently, then the Xvfb-wrapped Make
target after packaging. Diagnostic JSON/log files use a separate CI artifact;
the existing installer/update publication globs do not include them.
