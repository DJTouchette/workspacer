# Rust backend migration

The target is one Rust service library shared by a standalone executable and the
native GPUI host, following claudemon's owned-runtime model. The Go hub, brain,
MCP facade and launcher, and the private Node desktop-services companion, are
retiring implementations. External provider CLIs and enabled plugins remain
processes because they are external integrations.

## Completion criteria

- Existing clients retain their wire contracts, credential provenance, errors,
  timeouts, event ordering and subscription/demand behavior.
- Persisted config, credentials, session projections, layouts, jobs, federation,
  plugin settings and other state upgrade without losing data or identity.
- Local native operations use library calls/channels; network transports are
  adapters around the same services. The standalone executable owns signals;
  the GUI explicitly owns start/readiness/shutdown. Neither library installs
  process-wide signal handlers or exits its host.
- Every legacy source and regression suite has a replacement recorded in
  `services/hub-rs/migration.json`. This is an inventory, not a claim that a path
  reference alone proves equivalent behavior. Review and execute the tests.
- Native, Electron/web, TUI, remote workers, plugins and MCP work against Rust.
- CI, generators, packaging on all supported OSes, and deployment use Rust.
- Remove the Go backend only after these gates, retaining portable contract
  fixtures. No Go runtime fallback or hidden Node companion remains.

## Sequence and current state

The shared service graph and standalone launcher are implemented. The source-by-source review is complete:505 entries ported and29 explicitly
retired. Current work is final client/build cutover evidence, platform/package
validation and removal preparation. The14 cutover gates remain separate from
source-review completion.
Desktop, native local mode and TUI startup now select Rust. The current testing nightly, `0.169.0-nightly.202609291804`,
was published on 2026-09-29 from `9f786d734f996f487aa0894b8b5fae08aeba4ce3`.
It includes the native UI changes and modern MCP cache metadata fix. Later
reviewed parity fixes on main require a subsequent artifact.
This is explicitly an incomplete-migration preview. Native launches without
`--local` still attach to an existing service.

The original Go code remains a reference until the completion gates pass. The
manifest is deliberately conservative: implementation/test path mappings and
platform cutover checks must be reviewed before legacy removal. A retained
`migrationComplete:false` health field prevents interpreting this branch as a
completed migration.

The native Windows artifact uses the isolated Rust Preview identity/data folder.
Release packaging now selects that artifact and a standalone Rust server bundle.
[Release run 36609384642](https://github.com/DJTouchette/workspacer/actions/runs/36609384642)
successfully built Electron Windows/macOS/Linux packages, all three standalone
bundles and the Native Rust Preview Windows installer, including its install,
backend and uninstall smoke. The live nightly tag and assets were verified against
the candidate SHA. Native macOS/Linux installers are not part of that workflow.
The three default Docker image builds/fresh-volume boots passed earlier CI
checkpoints; final integrated revision checks remain required. See
[CUTOVER_STATUS.md](CUTOVER_STATUS.md) for all 14 gates and evidence boundaries.

Implemented migration slices include config and profile persistence, saved
sessions/layouts, usage preferences, filesystem access, Git review, search,
model catalogs, session projections/controls and an in-memory native client.
Scheduled jobs now have persistence, context guards, scheduling and execution
tests; MCP exposes a growing compatible subset through the Rust SDK's streamable
HTTP transport. Federation has owned outbound links, qualified calls with both
local and destination authorization, curated one-hop events, reconnection and
peer-file loading and owner-only configuration replacement. Native Rust mode
keeps GUI calls in memory and opens loopback adapters for external plugins and
provider MCP clients, with a persistent host credential.

Further integrated slices include plugin settings/installation/supervision and
HTTP routes, dynamic MCP plugin catalogs, worktree creation/removal, library
assets, session credential lifecycle, and routing/usage policy. Shared Go/Rust
routing and pacing fixtures guard policy semantics. These are substantial
migration slices, not evidence that the corresponding entire legacy packages
can yet be deleted. Local workflow/task admission and manager handoff coordination are now wired into
the owned runtime. Local agent spawning opens only after its coordinator, wake
scheduler, embedded engine and authenticated MCP facade are ready. A child-isolated
end-to-end test exercises launch through the real WebSocket adapter, a fake
provider process, authenticated MCP, first-message acknowledgement, progress and
completion wakes, and process cleanup. Paired remote dispatch now has durable receiver leases and restart tests. The
outbound execution-provider relay and its separate upstream MCP caller are
implemented; remaining source review and final integration evidence are part
of the pending federation and remote-worker gate.
Tests exercise the corresponding shared fixtures plus live temporary stores,
repositories, child processes and an embedded claudemon. This list does not
claim full parity for any unreviewed legacy file in the inventory.

Native integration uses the default `rust-hub` Cargo feature. A GUI build can
use `--rust-local-dir <isolated-directory>` to run the Rust services with no Go
children. Local launching is supported when the complete Rust service graph is ready;
unreviewed source inventory must not be interpreted as complete service parity.
`make test-native-rust` exercises the non-GUI controller/protocol suites and the
in-process native adapter. It is not a visual or cross-platform UI test.

## Repeatable commands

```sh
make test-hub-rust       # Rust runtime, transports, lifecycle and contract tests
make test-hub-parity     # same fixtures against the actual Go bus
make hub-migration      # inventory status and source-drift checks
python3 scripts/hub-migration.py backlog  # pending review groups, not missing implementation counts
python3 scripts/hub-migration.py backlog --json --prefix services/hub/cmd/brain/
make test-native-rust   # native protocol/controller + in-memory hub adapter
python3 scripts/hub-migration.py ready  # completion gate, currently fails
```

`make hub-vocabulary` exports the Go capability/topic/HTTP registry to a portable
contract. It is vocabulary coverage, not behavioral parity. Add shared fixture
cases before porting behavior; test the fixtures against both implementations.

For local core experiments, `wks-hub --listen 127.0.0.1:0 --token-file <file>`
uses an existing credential and prints its assigned address. Add `--database`,
`--config-dir`, `--data-dir`, and `--home-dir` to embed claudemon and the migrated
local services. No legacy process is launched. `Hub::start(Options::default())`
starts without sockets. `backend::Backend` owns both Rust engines.

## Earlier integration checkpoint (2026-09-28)

The combined crate passes `cargo check --all-targets` after wiring provider caller
proofs, the upstream MCP bridge, and the embedded conversation/statusline producer.
This is a compile checkpoint, not a complete suite or cutover claim. The provider
relay preserves the central hub / outbound worker topology; delegated calls carry
verified scope and a token fingerprint, never a bearer or synthesized host token.

Further implemented slices include threshold notifications, file-watch leases,
resumable-session discovery, review evidence, brief boards, analytics, terminal
leases, quiescence and machine-power policy, persistent push subscriptions and
HTTP/PWA assets. Native controller/protocol/embedding tests and a full GUI
feature compile have passed at earlier checkpoints; Windows process ownership
had compile-only coverage locally at that checkpoint. Subsequent Windows CI
and installer smoke passed; the testing nightly publication is described above.

The inventory remains deliberately conservative: unreviewed source rows and
cutover gates are pending even when a Rust module implements some behavior.
Packaging now selects the Rust backend. Do not delete the reference
implementation or announce migration completion until the compatibility and
packaging gates have been reviewed and run against a fixed source revision.

`make hub-capability-inventory` starts an isolated complete backend and compares
its actual installed methods with the reference brain/catalog and hub registries.
A mapped MCP name alone is not an implementation. The report explicitly does not
claim behavioral parity. `make build-rust-backend` builds the service used by the
normal Electron/TUI startup paths. Electron keeps its
existing provider services inside Electron and adopts the Rust hub's embedded MCP
listener; it does not start a separate Go facade or private Node companion.

The standard standalone configuration retains the old `workspacer-hub` directory
for layout, scheduled jobs/history, pacing preferences and push identity. Explicit
`--data-dir` overrides shared hub state; a custom `--config-dir` remains isolated.
The native preview continues to use its separate data directory.
