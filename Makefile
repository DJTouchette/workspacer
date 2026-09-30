# Workspacer monorepo — top-level orchestrator.
#
#   apps/desktop      Electron app (npm)        — the GUI client
#   apps/native       GPUI client (Rust/cargo)  — the native GUI
#   apps/tui          wks-tui (Rust/cargo)      — the terminal client
#   services/claudemon  Claude session daemon (Rust/cargo)
#   services/hub-rs     shared Rust backend and standalone launcher
#
# Each component also builds on its own from its directory; these targets just
# delegate so you have one entry point from the repo root.

DESKTOP   := apps/desktop
TUI       := apps/tui
NATIVE    := apps/native
CLAUDEMON := services/claudemon
HUB       := services/hub
HUB_RUST  := services/hub-rs

.PHONY: dev dev-share dev-tui run-tui install build build-desktop build-hub build-claudemon build-tui \
        build-cli test test-desktop test-hub test-tui test-claudemon test-routing-harness \
        claudemon-routes docs-drift package clean dev-native run-native build-native test-native \
        dev-native-local run-native-local build-native-local

## dev: run the desktop app in dev mode (Vite + Electron). Remote sharing is now
##      a runtime toggle (Remote control → Start sharing); use `make dev-share`
##      to force it on at launch instead.
##
## GPU is disabled by default in dev: Electron's GPU process segfaults 3x per
## launch in Mesa's libgallium (observed on Arch, Mesa 26.1.x, Wayland) before
## Chromium falls back to SwiftShader anyway — the crash-loop plus coredump
## processing is a big part of slow boots. This jumps straight to the fallback.
## Re-enable with `make dev WORKSPACER_DISABLE_GPU=0` to test real-GPU paths.
WORKSPACER_DISABLE_GPU ?= 1
dev:
	cd $(DESKTOP) && WORKSPACER_DISABLE_GPU=$(WORKSPACER_DISABLE_GPU) npm run dev

## dev-share: like `dev` but force-enables remote sharing at launch (env var),
##            for testing the web/bridged client without flipping the UI toggle.
dev-share:
	cd $(DESKTOP) && WORKSPACER_DISABLE_GPU=$(WORKSPACER_DISABLE_GPU) npm run dev:share

## dev-tui: run wks-tui (debug); builds the Rust services first. The TUI
##          defaults to the hub bus (auto-starting the Rust backend); pass
##          `ARGS="--direct"` for the standalone claudemon-direct path.
dev-tui: build-hub
	cd $(CLAUDEMON) && cargo build
	cd $(TUI) && cargo run -- $(ARGS)

## run-tui: run wks-tui (release); builds Rust services + tui first.
##          Defaults to the bus; `ARGS="--direct"` for claudemon-direct.
run-tui: build-claudemon build-hub build-tui
	cd $(TUI) && cargo run --release -- $(ARGS)

## dev-native: run the native GUI against the running desktop/headless hub.
##             ARGS="--bus wss://host/bus --token-file /path/to/token" for remote.
##             Demo data is opt-in only: ARGS="--demo".
##             Use dev-native-local to own an embedded backend instead.
dev-native:
	cargo run --locked --manifest-path $(NATIVE)/Cargo.toml -- $(ARGS)

## dev-native-local: run the native GUI with its shared in-process Rust backend.
##                   ARGS="--keep-running" minimizes the window on close.
##                   No standalone claudemon executable is needed.
dev-native-local:
	cargo run --locked --manifest-path $(NATIVE)/Cargo.toml -- --local $(ARGS)

## run-native: build and launch the release native GUI against the running hub.
run-native: build-native
	cargo run --locked --release --manifest-path $(NATIVE)/Cargo.toml -- $(ARGS)

## run-native-local: build and launch the release GUI with its embedded backend.
run-native-local: build-native-local
	cargo run --locked --release --manifest-path $(NATIVE)/Cargo.toml -- --local $(ARGS)

## build-native: build the native GUI binary (existing-hub or embedded mode).
build-native:
	cd $(NATIVE) && cargo build --locked --release --bin wks-native

## build-native-local: build the native binary and all required local services.
build-native-local: build-native

## test-native: native UI/protocol tests and embedded-engine lifecycle regression.
test-native:
	cd $(NATIVE) && cargo test --locked --features ui-tests
	cd $(CLAUDEMON) && cargo test --locked --test embedded

## install: install desktop JS dependencies
install:
	cd $(DESKTOP) && npm install

## build: build every component
build: build-hub build-claudemon build-desktop build-tui build-native

build-desktop:
	cd $(DESKTOP) && npm run build

build-hub: build-rust-backend

.PHONY: test-hub-rust test-hub-parity hub-migration hub-vocabulary test-native-rust hub-mcp-catalog
hub-mcp-catalog:
	python3 scripts/mcp-catalog.py --write

.PHONY: check-hub-mcp-catalog
check-hub-mcp-catalog:
	python3 scripts/mcp-catalog.py --check
	python3 -B -m unittest discover -s scripts -p 'test_mcp_catalog.py' -v
test-native-rust:
	cargo test --locked --manifest-path $(NATIVE)/Cargo.toml --no-default-features --features rust-hub
test-hub-rust:
	cargo test --locked --manifest-path $(HUB_RUST)/Cargo.toml

.PHONY: test-hub-latency
test-hub-latency:
	cargo test --locked --release --manifest-path $(HUB_RUST)/Cargo.toml --test bus_latency -- --ignored --nocapture

# Build the reference in a disposable directory. The environment variable is
# mandatory in the ignored test: this target never silently skips Go parity.
test-hub-parity: test-hub-rust
	@fixture_dir=$$(mktemp -d); trap 'rm -rf "$$fixture_dir"' EXIT; \
	(cd $(HUB) && go build -o "$$fixture_dir/hub-reference" ./cmd/hub-reference) && \
	WKS_GO_HUB_REFERENCE="$$fixture_dir/hub-reference" cargo test --locked --manifest-path $(HUB_RUST)/Cargo.toml --test compatibility shared_contracts_go_reference -- --ignored
	cd $(HUB) && go test ./cmd/brain -run '^Test(RustMigrationSnapshotFixtures|ContextHealthFormattingMatchesDesktopContract|ContextWatchRejectsUnsupportedProvidersWithoutUsingSlots|CumulativeCodexContractCannotFireContextWatch|TelemetryEpochKeepsAdjacentProductionValuesDistinct|ClaudeProjectDirNameContractCases|HeadlessFileWatch.*)$$' -count=1
	cd $(HUB) && go test ./internal/bus -run '^TestMigrationBusFixtures$$' -count=1
	cd $(HUB) && go test ./cmd/mcp -run '^TestRustMigrationToolCatalog$$' -count=1
	cd $(HUB) && go test ./internal/jobs -run '^TestRustMigrationJobFixtures$$' -count=1
	cd $(HUB) && go test ./internal/quiescence -run '^TestPortableFleetQuiescenceContract$$' -count=1
	cd $(HUB) && go test ./internal/routing ./internal/limits -run '^TestPortableRust(Routing|Pacing)Contract$$' -count=1
	cd apps/desktop && npm run test:main -- src/main/shared/structuredResult.test.ts src/main/shared/workerEscalation.test.ts src/main/shared/fleetMessages.test.ts src/main/services/thresholdWatch.test.ts
	@vocabulary=$$(mktemp); trap 'rm -f "$$vocabulary"' EXIT; \
	(cd $(HUB) && go run ./cmd/hub-reference --snapshot) > "$$vocabulary" && \
	cmp "$$vocabulary" $(HUB_RUST)/assets/hub-vocabulary.json

hub-migration:
	python3 scripts/hub-migration.py check

.PHONY: check-hub-capability-parameters
check-hub-capability-parameters:
	cargo fmt --manifest-path tools/capability-source-check/Cargo.toml --check
	cargo test --locked --manifest-path tools/capability-source-check/Cargo.toml
	cargo run --locked --manifest-path tools/capability-source-check/Cargo.toml -- --root . --check

.PHONY: check-hub-rust-windows-platform
check-hub-rust-windows-platform:
	cargo check --manifest-path tools/windows-contract-check/Cargo.toml --target x86_64-pc-windows-gnu --tests

.PHONY: check-store-yaml-parity
check-store-yaml-parity:
	@yaml_receipt=$$(mktemp); trap 'rm -f "$$yaml_receipt"' EXIT; \
	if ! cargo test --locked --manifest-path $(HUB_RUST)/Cargo.toml --features test-support --test provider_parity actual_saved_timestamps_are_quoted_and_roundtrip -- --nocapture > "$$yaml_receipt"; then \
	  cat "$$yaml_receipt"; exit 1; \
	fi; \
	node apps/desktop/scripts/check-store-yaml-parity.mjs "$$yaml_receipt"

.PHONY: check-hub-rust-assets
check-hub-rust-assets:
	node scripts/check-retained-plugin-assets.mjs
	python3 scripts/mcp-catalog.py --check
	python3 scripts/generate-rust-launch-assets.py --check
	python3 scripts/test-generate-rust-launch-assets.py
	node scripts/generate-rust-library-assets.mjs --check
	node scripts/generate-rust-workflow-assets.mjs --check
	node scripts/generate-rust-fleet-assets.mjs --check
	node scripts/generate-rust-brief-fixtures.cjs --check
	node scripts/generate-rust-asset-fixtures.cjs --check
	node scripts/generate-rust-workflow-artifact-fixtures.cjs --check
	node scripts/generate-rust-workflow-watcher-fixtures.cjs --check
	node scripts/generate-rust-brain-capabilities.cjs --check

hub-vocabulary:
	cd $(HUB) && go run ./cmd/hub-reference --snapshot > ../hub-rs/assets/hub-vocabulary.json

## build-cli: build the standalone shared Rust backend.
build-cli: build-rust-backend

build-claudemon:
	cd $(CLAUDEMON) && cargo build --release

build-tui:
	cd $(TUI) && cargo build --release

## test: run all test suites
test: test-desktop test-hub test-claudemon test-tui test-native

test-desktop:
	cd $(DESKTOP) && npm test

## test-hub: shared Rust backend and standalone CLI.
test-hub:
	cargo test --locked --manifest-path $(HUB_RUST)/Cargo.toml

## test-hub-reference: temporary Go oracle, retained until cutover validation.
test-hub-reference:
	cd $(HUB) && go test -race -count=1 ./...

test-claudemon:
	cd $(CLAUDEMON) && cargo test

test-tui:
	cd $(TUI) && cargo test

## test-routing-harness: real hub + fake claudemon runtime harness for
##                       limit-aware routing. Every assertion runs against the
##                       registered routing.select; set
##                       ROUTING_HARNESS_REQUIRE_ROUTING=1 to make a PARKED
##                       assertion (i.e. routing.select gone) fatal rather than
##                       a note.
test-routing-harness:
	env -u NO_COLOR node $(HUB)/scripts/routing-limit-harness.mjs

## claudemon-routes: regenerate contracts/claudemon-routes.json from the two
##                    axum routers in services/claudemon/src/daemon/{api,hook}.rs.
##                    The fixture is the SERVED half of the claudemon HTTP seam:
##                    capspec's caller sweep and apps/tui's mock_server both read
##                    it, so a hand-edited copy is a caller guard checking a list
##                    nobody serves. Run this after adding or deleting a route.
claudemon-routes:
	cd $(CLAUDEMON) && UPDATE_CLAUDEMON_ROUTES=1 cargo test --lib routes_contract

## docs-drift: informational scan for stale maturity wording in component READMEs
docs-drift:
	bash scripts/check-doc-drift.sh

## package: build daemons + produce desktop installers (electron-builder)
package:
	cd $(DESKTOP) && npm run package

## clean: remove build artifacts across components
clean:
	rm -rf $(DESKTOP)/dist $(DESKTOP)/release
	rm -f $(HUB)/hub $(HUB)/hub.exe $(HUB)/mcp $(HUB)/mcp.exe $(HUB)/brain $(HUB)/brain.exe \
	      $(HUB)/workspacer $(HUB)/workspacer.exe $(HUB)/claudemon $(HUB)/claudemon.exe
	cd $(CLAUDEMON) && cargo clean
	cd $(TUI) && cargo clean
	cd $(NATIVE) && cargo clean

.PHONY: hub-mcp-inventory
hub-mcp-inventory:
	cargo run --manifest-path $(HUB_RUST)/Cargo.toml --bin wks-hub -- --mcp-inventory

.PHONY: hub-capability-inventory
hub-capability-inventory:
	cargo run --manifest-path $(HUB_RUST)/Cargo.toml --example capability_inventory

.PHONY: build-rust-backend
build-rust-backend:
	cargo build --release --manifest-path $(HUB_RUST)/Cargo.toml --bin workspacer-rust

.PHONY: test-tui-rust-backend
## test-tui-rust-backend: exercise the TUI client against an already-built real Rust backend.
# Explicit probe; normal TUI unit tests do not require a backend executable.
test-tui-rust-backend:
	@test -n "$(WKS_RUST_BACKEND_BIN)" || (echo "Set WKS_RUST_BACKEND_BIN to an absolute built workspacer-rust executable" >&2; exit 1)
	WKS_RUST_BACKEND_BIN="$(WKS_RUST_BACKEND_BIN)" cargo test --locked --manifest-path $(TUI)/Cargo.toml daemons::backend_smoke::real_rust_backend_calls_events_reconnect_and_owned_shutdown -- --ignored --exact --nocapture

.PHONY: test-electron-ownership
## test-electron-ownership: Linux/Xvfb ownership probe of an already-packaged Electron app.
test-electron-ownership:
	@test -n "$(WKS_ELECTRON_EXECUTABLE)" -a -n "$(WKS_ELECTRON_SMOKE_REPORT)" || (echo "Set absolute WKS_ELECTRON_EXECUTABLE and WKS_ELECTRON_SMOKE_REPORT paths" >&2; exit 1)
	node apps/desktop/scripts/smoke-electron-ownership.mjs "$(WKS_ELECTRON_EXECUTABLE)" "$(WKS_ELECTRON_SMOKE_REPORT)"

.PHONY: check-retained-plugin-assets
check-retained-plugin-assets:
	node scripts/check-retained-plugin-assets.mjs
