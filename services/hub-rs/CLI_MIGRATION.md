# Launcher contract review

The installed command is `workspacer`; its shipped executable remains
`workspacer-rust`. The launcher owns the Rust backend in process. It no longer
locates or restarts separate hub, brain, MCP, or session-daemon executables.
Old `--hub-bin`, `--brain-bin`, `--mcp-bin`, and `--claudemon-bin` options fail
with an ownership explanation rather than choosing a legacy executable.

## Retained boundaries

- Existing command groups, help/errors and known single-dash long options are
  retained. Option values and arguments after `--` are not reinterpreted.
  Explicit empty plugin directories disable loading; empty web/DB/install
  directory choices preserve their documented fallback semantics.
- Explicit session DB paths win. Otherwise absolute XDG data is preferred, then
  the selected home. Empty/relative inferred paths are refused. Either changed
  daemon port requires an explicit database. Home selection uses an explicit
  home option, environment, then the OS account; planning does not create it.
- CLI `usage.pollOnBoot` follows the Go launcher's config-first rule: only a raw
  boolean becomes an engine option. Absent/malformed values leave environment
  and engine defaults intact. Native's environment-first behavior is separate.
- Owner identity loss is refused. Explicit empty `--token` clears the ambient
  token in favor of persisted identity; explicit `--allow-new-token=false`
  overrides the ambient mint opt-in. No flag value may become a second option.
- Ready banners retain all endpoint keys and URL-encoded pairing links. Quiet
  startup prints neither JSON nor text secrets. The facade readiness check pins
  its service, Rust implementation, listener, upstream hub and initial catalog.
- Plugin development validates before identity/hooks/startup, isolates its
  source from installed plugins, waits for a quiet scan, builds before reload,
  and keeps the live plugin when the build fails. Loader markers and dependency
  trees do not cause rebuild loops; symlinks are observed without recursion.
  Lifecycle logging is best-effort and does not own the file watcher lifetime.
- SIGTERM/interrupt, stdin EOF and a declared parent's death terminate the owned
  graph. A borrowed daemon's hook ports, storage and lifetime remain external.

## Explicit borrowed-daemon change

`serve --external-claudemon` still accepts a bare flag, resolving to loopback at
`--claudemon-api-port`. It now requires **explicit `--hub-only`**; an optional URL
selects the same read-only integration. Its exact maintenance health marker and
body must be present before startup continues. The old full-stack mode attached
Go/Node launch controllers to an externally owned daemon. That process topology
is retired, not silently converted into a partial owned backend. Full standalone
and embedded operation own their Rust engine.

## Per-file evidence responsibility

| Go launcher files | Rust replacement and evidence |
| --- | --- |
| `main.go` | `cli/mod.rs`, `cli/compat.rs`, `bin/workspacer-rust.rs`; command/alias/value/boolean/unsupported-child-override checks in compatibility units and `tests/cli.rs` |
| `plan.go`, `plan_test.go` | `cli/serve.rs`, `cli/presentation.rs`, `net_address.rs`; real standalone lifecycle, strict facade health, encoded endpoint/banner, plugin-origin and trusted-host tests |
| `resolve.go`, `resolve_test.go` | Child lookup retires; `cli/launcher_paths.rs` preserves shipped web discovery and `cli/install.rs` publishes the real Rust executable. Fake directory fixtures and standalone startup replace executable-name scanning |
| `serve.go` | `cli/serve.rs`, `backend.rs`, owned Hub/runtime; foreground startup/shutdown, collision, health, hooks, identity and parent lifetime tests. Restarting separately missing children is intentionally absent |
| `servedb_test.go` | `cli/launcher_paths.rs`, `cli/identity.rs`, `ServePlan`; pure path matrix, subprocess relative/empty environment refusal, OS-home fallback and real pinned engine store lifecycle |
| `serveinit_test.go` | Library hook initialization before backend startup and explicit opt-out; real CLI temporary-home hooks plus `tests/backend_hooks.rs`. No `claudemon init` child is launched |
| `external_claudemon_test.go` | `cli/readiness.rs`, explicit `hub-only` plan and read-only adapter; fake HTTP identity/redirect matrix, real CLI attach/no writes, no DB/hooks ownership, owned-port collision and graceful shutdown while external HTTP remains live |
| `dialhost_test.go` | `net_address.rs`, CLI authority/ready metadata; wildcard and concrete IP mapping, actual listener/plugin callback tests and real trusted-host request checks |
| `plugindev.go`, `plugindev_test.go` | `cli/dev_watch.rs`, `cli/serve.rs`, plugin manager; deterministic quiet-scan/ignore tests plus real CLI test-executable build success/failure, isolated plugin list, reload and source-preserving cleanup |
| `usagecfg.go`, `usagecfg_test.go` | `cli/usage.rs` → `ServePlan` → `EngineOptions`; raw bool/missing/malformed matrix and subprocess ambient-env precedence test without executing a provider |

Final ledger records are reviewed per file. Linux execution does not stand in
for Windows/macOS runtime CI: portable EOF/quiet/PID tests are now enabled there,
but their final-head platform run is a separate release gate.
