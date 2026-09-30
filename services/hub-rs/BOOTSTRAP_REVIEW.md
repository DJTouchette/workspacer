# Hub bootstrap review

This review covers all 1,329 lines of the retiring `cmd/hub/main.go`, including
its HTTP helpers. The final actual CLI checkpoint passed 22 tests with no
failures or ignored tests (`/tmp/workspacer-bootstrap-cli-final.log`).
The pure resolver helper suite also passed all three tests with no failures
or ignored tests (`/tmp/workspacer-provider-cli-helper.log`). Exact source
hash and migration evidence are in `reviews/cmd-hub-main.json`.

| Legacy responsibility | Current owner and composition evidence |
| --- | --- |
| Broker, identity, scoped credentials, trusted proxy names | `runtime.rs::Hub::start` validates configuration before starting its dedicated actor runtime; `server.rs` and `server/policy.rs` apply credentials and Host/Origin checks at the actual listener. `http.rs` integration tests exercise the real adapter. Network listeners now require a host token; anonymous component startup is retired. |
| Internal self-dial for jobs/quiescence/uploads | Owned `Handle` and internal `Client::connect_service` replace a TCP loopback credential channel. Runtime installs uploads, jobs, quiescence and remote administration before readiness. Internal identity remains distinct from a person's connection. |
| Layout and Overview pacing persistence | `services::install` installs layout and pacing handlers using the selected data directory. Standard installs retain the historical workspacer-hub state directory; explicit config/data roots stay isolated. |
| Routing, shared quota sampler, spawn ceiling and audit | Runtime creates one routing owner before installing handlers and spawn services. See `CMD_HUB_ROUTING_REVIEW.md` and the source-level bus admission records for detached fetch lifetime, catalog checks, qualified admission and audit ownership. |
| Node methods despite absent registry | `services/nodes/mod.rs` installs an empty list and explicit unknown-node wake/sleep responses without polling. The actual full-backend headless inventory calls all three methods, rather than proving only a helper. Root corrected this composition omission in e7504f49. |
| Federation peers/configuration | Runtime loads the selected file, starts one manager, installs owner-only configuration and resume handling, and joins it at shutdown. The legacy argv credential parser `-peer` is retired; current launchers use peers.json and owner APIs. |
| Web Push | `services/push::install` receives the host/scoped credential validator, opens private persistent state, installs handlers and starts the observer. Initialization failure logs and disables push without preventing readiness. The observer is explicitly stopped and joined. |
| Plugin HTTP routes, SDK, static UI, settings and install lifecycle | Runtime builds the plugin manager and policy-bound HTTP router, seeds safe examples, then loads plugins after actor readiness. `plugin_manager.rs` exercises real HTTP authority, private settings, UI containment, install consent and process shutdown. Static SDK/origin routes remain in the plugin router. |
| Remote/mobile/PWA/full web assets | `server/web.rs` owns the routes. Remote/full-app entry pages require operator credentials; static/mobile bootstrap remains public. HTTP host policy wraps the whole server. The CLI process fixture probes public assets and private entry pages; `tests/http.rs` checks host policy across the route registry. |
| Brain and claudemon siblings | Full standalone uses `Backend` with an owned Rust engine and hub; there is no separate brain child. Catalog/control-plane mode is explicit `--hub-only`; borrowed daemon observation requires `--external-claudemon` and identity/readiness verification. Legacy child binary overrides fail with a clear ownership explanation. |
| SSE bridge | Owned-engine sessions use the embedded event owner; hub-only borrowed engines use the explicitly configured external adapter. A free-standing arbitrary `--claudemon-events` stream is no longer a second independent source of truth. |
| Process confinement and parent death | Plugin children use the shared suspended-launch Windows job owner; PTY children have their own containment. This replaces confining the embedding GUI host in one global job. `cli/parent.rs` watches declared parent PID and stdin EOF; CLI process fixtures verify both signals independently. |
| Startup/readiness/failure cleanup | `cli/serve.rs` preflights selected ports, retains ownership outside cancellable initialization, waits for the actual MCP catalog, and cleans up failed startup. `Backend::shutdown` joins hub before engine and retains database ownership if cleanup is uncertain. CLI and backend ownership tests use isolated roots/processes. |
| Shutdown | Actor shutdown keeps the bus pumping while plugins and dispatch receivers stop, closes/drains queued commands before joining observers, then joins owned services, tasks and transports. `lifecycle.rs`, plugin shutdown, the visible-terminal watchdog and queue-drain regression prove the actual ordering. A second launcher signal may force exit; clean shutdown is explicitly awaited. |

## Flag and ownership changes

The shipped Electron launcher (`apps/desktop/src/main/services/hubDaemon.ts`)
and Fly role launcher (`deploy/fly/rust/launch.sh`) invoke `workspacer-rust
serve`. They no longer invoke the old `hub` component or depend on its
`--addr`, `--brain-scope`, or per-component file flags. The isolated legacy
supervisor under `deploy/fly/rust/fixtures` is not copied into the Rust images.

- `--addr` becomes the existing launcher `--host` and `--hub-port` pair.
- `--brain-scope`/`--brain-bin` become owned full mode, explicit hub-only mode,
  or upstream `--provider-scope`; no redundant brain subprocess is restored.
  `--brain-mcp-facade` is replaced by the owned facade listener/readiness
  configuration (`--mcp-port`/`--no-mcp` and explicit MCP access policy).
- `--claudemon`/`--claudemon-events` become owned engine configuration or an
  explicitly borrowed read-only daemon integration.
- `--layout-file`, `--routing-file`, and `--usage-prefs-file` component options
  are replaced by launcher configuration/data directories. The full standalone
  owner is persistent; an arbitrary per-component stateless layout/routing
  mode is not advertised as preserved.
- `--plugins-stream-logs` is controlled by `plugin dev`; plain serve remains
  quiet. `--plugins-dir`, `--examples-dir`, `--sidecar-node`, `--plugin-origin`,
  `--trusted-host`, `--peers-file`, `--nodes-file`, `--push-dir`, `--jobs-file`,
  `--tokens-file` and `--webapp-dir` remain launcher selections.
  `--nodes-keep-failed-wakes-running` still passes directly to the node owner;
  its default remains false, preserving stop-on-failed-wake behavior.

## Corrected composition edges

The webapp resolver previously discarded an explicit empty path and then used
the environment or discovered bundle. The narrow fix now makes explicit empty
disable the full app while omission retains environment/discovery defaults.
The existing real CLI fixture now checks environment-selected 200, explicit
empty 404 and explicit-path 200. A pure selection test also proves discovery
is not invoked after explicit disable. All actual CLI controls and the isolated discovery-closure helper passed.

Clap's default path parser rejected historical empty disable arguments. The
launcher now accepts empty nodes/push/examples/jobs/peers selections only where
the corresponding behavior is defined. Nodes retain an empty API without
polling; push has no observer or methods; examples are normalized to no source
before discovery or seeding (an empty Path joined to `editor` would otherwise
read the working directory). Jobs and peers normalize to disabled owners rather
than attempting persistence to an empty filename. The CLI fixture uses malformed
default registry files as negative controls, calls the actual node/peer methods,
and checks disabled method inventory with enabled positive controls.

The old component's `--tokens-file ""` disable is intentionally not restored
for the owned standalone launcher: its session launch/MCP ownership needs a
credential store. Empty remains rejected, and network host authentication is
unchanged. This is distinct from disabling background services.

Electron now passes an explicit empty webapp argument when sharing is off or
its selected web build is absent. Exact argv tests cover both cases and the
enabled path. The three owning hubDaemon suites passed 39 tests and main
TypeScript typechecking passed. The Rust CLI checkpoint passed 22 tests.

The plugin origin endpoint again sends explicit `Cache-Control: no-cache`,
matching the legacy bootstrap helper. The actual CLI HTTP fixture asserts
that header; no route or authority was changed.

The helper receipt is scoped: the same compiled unit binary separately
reproduced an unrelated provider re-registration gap before its correction.
This review does not claim a full current library/CI pass, Windows execution,
or any cutover gate completion.
