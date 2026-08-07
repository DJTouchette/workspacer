# Workspacer monorepo — top-level orchestrator.
#
#   apps/desktop      Electron app (npm)        — the GUI client
#   apps/tui          wks-tui (Rust/cargo)      — the terminal client
#   services/claudemon  Claude session daemon (Rust/cargo)
#   services/hub        control-plane / event bus (Go)
#
# Each component also builds on its own from its directory; these targets just
# delegate so you have one entry point from the repo root.

DESKTOP   := apps/desktop
TUI       := apps/tui
CLAUDEMON := services/claudemon
HUB       := services/hub

.PHONY: dev dev-share dev-tui run-tui install build build-desktop build-hub build-claudemon build-tui \
        build-cli test test-desktop test-hub test-tui test-claudemon docs-drift package clean

## dev: run the desktop app in dev mode (Vite + Electron). Remote sharing is now
##      a runtime toggle (Remote control → Start sharing); use `make dev-share`
##      to force it on at launch instead.
dev:
	cd $(DESKTOP) && npm run dev

## dev-share: like `dev` but force-enables remote sharing at launch (env var),
##            for testing the web/bridged client without flipping the UI toggle.
dev-share:
	cd $(DESKTOP) && npm run dev:share

## dev-tui: run wks-tui (debug); builds claudemon + hub/brain first. The TUI now
##          defaults to the hub bus (auto-spawning the hub + brain); pass
##          `ARGS="--direct"` for the standalone claudemon-direct path.
dev-tui: build-hub
	cd $(CLAUDEMON) && cargo build
	cd $(TUI) && cargo run -- $(ARGS)

## run-tui: run wks-tui (release); builds release claudemon + hub/brain + tui first.
##          Defaults to the bus; `ARGS="--direct"` for claudemon-direct.
run-tui: build-claudemon build-hub build-tui
	cd $(TUI) && cargo run --release -- $(ARGS)

## install: install desktop JS dependencies
install:
	cd $(DESKTOP) && npm install

## build: build every component
build: build-hub build-claudemon build-desktop build-tui

build-desktop:
	cd $(DESKTOP) && npm run build

build-hub:
	cd $(HUB) && go build -o . ./cmd/hub && go build -o . ./cmd/mcp && go build -o . ./cmd/brain && go build -o . ./cmd/workspacer

## build-cli: build the headless-server launcher (`workspacer serve`) plus the
##            daemons it supervises, all as siblings in services/hub/ so the
##            launcher's sibling-first binary resolution finds them.
build-cli: build-hub build-claudemon
	cp $(CLAUDEMON)/target/release/claudemon $(HUB)/claudemon

build-claudemon:
	cd $(CLAUDEMON) && cargo build --release

build-tui:
	cd $(TUI) && cargo build --release

## test: run all test suites
test: test-desktop test-hub test-claudemon test-tui

test-desktop:
	cd $(DESKTOP) && npm test

## test-hub: the Go suite. -count=1 is the second belt on the cross-repo guards.
##           Much of services/hub's suite reads files ABOVE the module —
##           apps/desktop/src/main/services/hubCapabilities.ts, contracts/*.json,
##           this Makefile, .github/workflows/ci.yml — and cmd/go's test cache
##           drops out-of-module inputs from the cache key
##           (computeTestInputsID: "Do not recheck files outside the module").
##           internal/extinput puts them back by reading through a path that
##           still descends lexically from the module root, which covers the
##           edits that change a file's size or mtime; it does NOT cover a
##           listing that grew a file where nothing was read, and cmd/go never
##           hashes contents for these. The whole module runs uncached in 7s, so
##           there is no reason to pay that risk for a saving nobody notices.
##           cmd/brain's TestTheTestCacheBeltIsInCIAndTheMakefile asserts this
##           flag is still here.
test-hub:
	cd $(HUB) && go test -race -count=1 ./...

test-claudemon:
	cd $(CLAUDEMON) && cargo test

test-tui:
	cd $(TUI) && cargo test

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
