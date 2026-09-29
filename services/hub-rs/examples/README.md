# Contract fixtures

`hub_contract_fixture` is available only with the `test-support` Cargo feature.
It runs the production Rust bus, HTTP/MCP transports, authentication, plugin
routes and remote dispatch protocol. It is not a distributable backend.

```sh
cargo build --manifest-path services/hub-rs/Cargo.toml \
  --features test-support --example hub_contract_fixture
cd apps/desktop
npm run test:dispatch-chain
npx playwright test --project=app --project=mobile --workers=1
```

The TypeScript fixtures build and locate the executable through Cargo's JSON
artifact output, respecting `CARGO_HOME`, `CARGO_TARGET_DIR` and
`CARGO_INCREMENTAL`. The build gets its own setup budget and is cached within
one test worker. A suite coordinator can set `WKS_TEST_HUB_FIXTURE_BINARY` to
the absolute compiler artifact path after one build, avoiding build-lock waits
across Playwright workers. Runtime HOME/XDG directories and every state path
are isolated.

Modes:

- `browser`: actual HTTP assets and bus with test capability providers attached
  over WebSocket; optional cloud URLs must point to a loopback fake server.
- `mcp`: actual MCP facade and caller credentials, with an Electron test provider
  and deterministic routing inputs. Requires `WKS_DISPATCH_CHAIN_FIXTURE=1`.
- `paired`: actual receiver leases, replay, return events, progress and wake
  validation, with an in-memory execution boundary and real temporary Git
  worktrees. Requires `WKS_PAIRED_CHAIN_FIXTURE=1`. The loopback control/evidence
  endpoints exist only in this test executable.

The compiler wrapper owns its Cargo process group and terminates descendants
on deadline, cancellation or parent loss. Run its focused regression with
`node --test apps/desktop/tests/e2e/fixtures/rustBuildProcess.test.cjs`. Windows
uses `taskkill /T /F` before retiring the Cargo root; Windows crash and process
ownership execution remains part of the platform CI gate.

All modes require an explicit scratch root, emit one readiness JSON line,
shut down on parent stdin EOF, and have a ten-minute orphan backstop. No real
provider CLI or cloud mutation is part of these tests. Default Cargo builds
exclude the test support module and its private actor commands.

On a Debian 13 development container without root, missing Chromium libraries
can be extracted with `tools/playwright-user-deps.sh /absolute/scratch/path`.
The helper prints the `LD_LIBRARY_PATH` prefix for the browser test command;
it does not install system packages. CI should use Playwright's normal
`install --with-deps chromium` setup instead.
