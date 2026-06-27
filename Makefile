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

.PHONY: dev dev-tui run-tui install build build-desktop build-hub build-claudemon build-tui \
        test test-desktop test-hub test-tui package clean

## dev: run the desktop app in dev mode (Vite + Electron) with remote sharing on
dev:
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
	cd $(HUB) && go build -o . ./cmd/hub && go build -o . ./cmd/mcp && go build -o . ./cmd/brain

build-claudemon:
	cd $(CLAUDEMON) && cargo build --release

build-tui:
	cd $(TUI) && cargo build --release

## test: run all test suites
test: test-desktop test-hub test-tui

test-desktop:
	cd $(DESKTOP) && npm test

test-hub:
	cd $(HUB) && go test -race ./...

test-tui:
	cd $(TUI) && cargo test

## package: build daemons + produce desktop installers (electron-builder)
package:
	cd $(DESKTOP) && npm run package

## clean: remove build artifacts across components
clean:
	rm -rf $(DESKTOP)/dist $(DESKTOP)/release
	rm -f $(HUB)/hub $(HUB)/hub.exe $(HUB)/mcp $(HUB)/mcp.exe $(HUB)/brain $(HUB)/brain.exe
	cd $(CLAUDEMON) && cargo clean
	cd $(TUI) && cargo clean
