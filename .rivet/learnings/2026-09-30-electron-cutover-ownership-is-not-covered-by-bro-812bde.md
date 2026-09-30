---
title: Electron cutover ownership is not covered by browser or standalone archive smoke
date: 2026-09-30
confidence: high
suggested_doc: hub-process-supervision
related_paths:
  - apps/desktop/src/main/index.ts
  - apps/desktop/src/main/services/hubDaemon.ts
  - .github/workflows/release.yml
promoted: false
---

# Electron cutover ownership is not covered by browser or standalone archive smoke

## Observation
Current desktop CI runs Chromium app/mobile/renderer projects against real Rust fixtures, while dispatch-chain mocks Electron and hubDaemon unit tests mock child_process/daemonUtils. Opt-in libraryPane launches real Electron but asserts UI/files, not Rust readiness or listener retirement. Release36609384642 at9f786d734f996f487aa0894b8b5fae08aeba4ce3 passed all3 platform standalone archive smokes and Native Windows install/backend/uninstall; those scopes do not exercise normal Electron main startup→hubDaemon real Rust child→before-quit cleanup. Existing WORKSPACER_PORT_OFFSET and isolated package roots permit a narrow ownership probe without new product flags or visual-GUI criteria.

## Recommendation
Use an explicitly invoked already-built Electron artifact ownership harness: actual authenticated Rust/MCP and desktop registration, normal app.close, child/listener retirement and adopted external-server survival. Preserve archive and native Preview installer receipts as their own exact-source scopes.

## Harness implementation checkpoint
The explicit Linux/Xvfb packaged ownership script and Make target now exist.
Local artifact/Xvfb/standalone claudemon prerequisites are absent, so only helper
validation is claimed until the authorized release preview executes it. Installed
Playwright coreBundle confirms app.close requests app.quit but outer failed-close
cleanup can kill the process. The harness explicitly requests normal app.quit,
requires actual Electron code0/signalnull and exact listener retirement, and
never uses failure cleanup as successful ownership evidence. Adopted external
Rust service survival is a separate positive control before its own EOF shutdown.

## Release wiring refinement
Linux release runs helpers independently after packaging, then xvfb-run around
the named Make target, reusing linux-unpacked resources. Reports/logs go to a
separate CI artifact and are excluded from installer publication globs. MCP
health checks exact Rust identity, connected/catalog flags and both endpoints;
the captured current operator fixture has100 unique tools, so the smoke requires
at least100 unique nonempty names plus get_host_cwd. Five helper regressions and
actionlint passed; real packaged execution still awaits the root-owned release.
