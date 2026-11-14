# Workspacer monorepo — top-level orchestrator.
#
#   apps/native       GPUI client (Rust/cargo)  — the app: what `make dev`,
#                                                 `make package` and releases build
#   apps/desktop      Electron app (npm)        — deprecated (last release
#                                                 v0.170.0); its renderer is still
#                                                 the /app web client. Reach it
#                                                 through the *-desktop targets.
#   apps/tui          wks-tui (Rust/cargo)      — the terminal client
#   services/claudemon  Claude session daemon (Rust/cargo)
#   services/hub-rs     shared Rust backend and standalone launcher
#
# The release version is the root VERSION file (see scripts/check-version.mjs).
#
# Each component also builds on its own from its directory; these targets just
# delegate so you have one entry point from the repo root.

DESKTOP   := apps/desktop
TUI       := apps/tui
NATIVE    := apps/native
CLAUDEMON := services/claudemon
HUB       := services/hub
HUB_RUST  := services/hub-rs

.PHONY: dev dev-desktop dev-share dev-tui run-tui install install-native install-desktop \
        build build-desktop build-hub build-claudemon build-tui \
        build-cli test test-desktop test-hub test-tui test-claudemon test-routing-harness \
        claudemon-routes docs-drift package package-native package-desktop clean \
        dev-native run-native build-native test-native \
        dev-native-local run-native-local build-native-local bench-native-switch \
        package-native-linux package-native-macos package-native-windows check-version \
        landing-shots landing-plugin-md

## dev: run the native app (debug build) with its embedded Rust backend.
##      Same as `make dev-native-local`; nothing else has to be running.
##      ARGS="--keep-running" minimizes the window on close.
dev: dev-native-local

## dev-desktop: run the deprecated Electron app in dev mode (Vite + Electron).
##              Needs `make install-desktop` first. Remote sharing is a runtime
##              toggle there (Remote control → Start sharing); use
##              `make dev-share` to force it on at launch instead.
##
## GPU is disabled by default in dev: Electron's GPU process segfaults 3x per
## launch in Mesa's libgallium (observed on Arch, Mesa 26.1.x, Wayland) before
## Chromium falls back to SwiftShader anyway — the crash-loop plus coredump
## processing is a big part of slow boots. This jumps straight to the fallback.
## Re-enable with `make dev-desktop WORKSPACER_DISABLE_GPU=0` to test real-GPU paths.
WORKSPACER_DISABLE_GPU ?= 1
dev-desktop:
	cd $(DESKTOP) && WORKSPACER_DISABLE_GPU=$(WORKSPACER_DISABLE_GPU) npm run dev

## dev-share: launches the ELECTRON dev loop (`dev-desktop`), not the native
##            app, with remote sharing forced on at launch, for testing the
##            web/bridged client without flipping the UI toggle. It works by
##            setting WORKSPACER_REMOTE_SHARE=1, which only the Electron host reads.
dev-share:
	@echo "dev-share: starting the Electron dev loop (apps/desktop) with remote sharing forced on"
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

## dev-native: run the native GUI against an already running hub.
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

## package: build the native app's download for this OS into
##          apps/native/target/package/ (Linux tar.gz, macOS .dmg, Windows
##          installer). The Electron installers are `make package-desktop`.
package: package-native

package-native:
	@case "$$(uname -s)" in \
	  Linux) $(MAKE) package-native-linux ;; \
	  Darwin) $(MAKE) package-native-macos ;; \
	  *) $(MAKE) package-native-windows ;; \
	esac

## package-native-linux: release build packed as a tar.gz with a per-user
##             installer (apps/native/target/package/). Linux only.
package-native-linux:
	$(NATIVE)/scripts/package-linux.sh

## package-native-macos: release build as Workspacer.app in a .dmg
##             (apps/native/target/package/). macOS only.
package-native-macos:
	$(NATIVE)/scripts/package-macos.sh

## package-native-windows: Workspacer-Setup-<version>-x64.exe
##             (apps/native/target/package/). Windows x64 only; needs NSIS and
##             NATIVE_CRT_DIR (see apps/native/scripts/package-windows.mjs).
package-native-windows: build-native build-rust-backend
	node $(NATIVE)/scripts/package-windows.mjs

## check-version: VERSION and apps/desktop/package.json must agree.
check-version:
	node scripts/check-version.mjs

## landing-shots: restage the marketing page's native captures (landing/shots/native-*.webp)
##             from the fixture hub. Linux only; needs Xvfb, openbox, xdotool and cwebp.
landing-shots:
	cd $(NATIVE) && cargo build --locked --bin wks-native --bin native-harness
	landing/capture-shots.sh

## landing-plugin-md: regenerate landing/build-plugin.md, the plain-text twin of
##             landing/build-plugin.html. Edit the HTML, then run this.
landing-plugin-md:
	python3 landing/gen-build-plugin-md.py landing/build-plugin.html > landing/build-plugin.md.tmp
	mv landing/build-plugin.md.tmp landing/build-plugin.md

## test-native: native UI/protocol tests and embedded-engine lifecycle regression.
test-native:
	cd $(NATIVE) && cargo test --locked --features ui-tests
	cd $(CLAUDEMON) && cargo test --locked --test embedded

## bench-native-switch: parent/child chat switching cost, dense fixtures.
##                      Controller latency + hub reads per switch, then the
##                      GPUI update/frame cost (test platform: no text shaping
##                      or GPU). PROFILE=release for an optimized build;
##                      the dev profile is unoptimized and much slower.
PROFILE ?= dev
bench-native-switch:
	cd $(NATIVE) && cargo run --locked --profile $(PROFILE) --bin native-harness -- bench-switch
	cd $(NATIVE) && WKS_NATIVE_BENCH_SWITCH=10 cargo test --locked --profile $(PROFILE) --features ui-tests --bin wks-native bench_switch_frames -- --nocapture

## install: build the native app and install it for the current user
##          (Linux: ~/.local, or PREFIX=...; removable with wks-native-uninstall).
##          The native app has no dependencies to install first. On macOS and
##          Windows use `make package` and open the .dmg / run the installer.
INSTALL_STAGE := $(NATIVE)/target/package/install-stage
install: install-native

install-native:
	@case "$$(uname -s)" in \
	  Linux) ;; \
	  *) echo "make install is Linux-only: run 'make package' and open the result from apps/native/target/package/" >&2; exit 1 ;; \
	esac
	@set -e; \
	archive=$$($(NATIVE)/scripts/package-linux.sh | tail -n 1); \
	rm -rf $(INSTALL_STAGE); mkdir -p $(INSTALL_STAGE); \
	tar -xzf "$$archive" -C $(INSTALL_STAGE); \
	(cd $(INSTALL_STAGE)/workspacer-*-linux-* && ./install.sh); \
	rm -rf $(INSTALL_STAGE)

## install-desktop: install the Electron app's JS dependencies (needed by
##          dev-desktop, build-desktop, test-desktop and package-desktop).
install-desktop:
	cd $(DESKTOP) && npm install

## build: build every component
build: build-hub build-claudemon build-desktop build-tui build-native

build-desktop:
	cd $(DESKTOP) && npm run build

build-hub: build-rust-backend

.PHONY: test-hub-rust test-hub-reference test-hub-parity hub-migration hub-vocabulary test-native-rust hub-mcp-catalog
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

# Optional historical oracle: validation fails before any compiler runs unless
# WKS_HUB_REFERENCE_ROOT names the pinned, clean historical repository checkout.
test-hub-parity:
	python3 scripts/hub-reference.py parity

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
	python3 scripts/gen-mobile-tokens.py --check
	python3 scripts/test-gen-mobile-tokens.py
	node scripts/check-mobile-next-assets.mjs
	node scripts/test-mobile-next-fleet.mjs
	node --no-warnings scripts/test-mobile-next-bus.mjs

# Read-only comparison; explicit export is available through hub-reference.py.
hub-vocabulary:
	python3 scripts/hub-reference.py vocabulary-check

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

## test-hub-reference: explicit pinned historical Go oracle, never a live backend.
test-hub-reference:
	python3 scripts/hub-reference.py test

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
	python3 scripts/hub-reference.py routing-harness

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

## package-desktop: build daemons + produce the deprecated Electron installers
##          (electron-builder, into apps/desktop/release/). Not part of a release.
package-desktop:
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
