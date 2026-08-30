---
title: A worktree's renderer node_modules must symlink to the MAIN CHECKOUT's src/renderer/node_modules, not to apps/desktop/node_modules
date: 2026-08-23
promoted: false
dropped: true
dropped_reason: superseded — worktreeService.discoverNodeModules now recurses to any depth, so the renderer node_modules symlink is created automatically (verified on this worktree). The depth fix is already recorded in domains/agent-spawn.md.
---

# A worktree's renderer node_modules must symlink to the MAIN CHECKOUT's src/renderer/node_modules, not to apps/desktop/node_modules

## Observation
apps/desktop/src/renderer has its OWN node_modules with its own dependency set (@vitejs/plugin-react lives there and NOT in apps/desktop/node_modules). In a git worktree, apps/desktop/node_modules is already a symlink to the main checkout's copy, so the obvious hand-fix — `ln -s ../../node_modules src/renderer/node_modules` — resolves to apps/desktop/node_modules and the renderer vitest config fails to load: "Cannot find package '@vitejs/plugin-react'", reported against a `.vite-temp/` path inside the MAIN checkout, which makes it look like a worktree-escape bug rather than a missing package. `npx vitest run` also silently reports "No test files found" if invoked from apps/desktop with a renderer test path — the renderer suite's include globs are relative to src/renderer, so it must be run as `cd src/renderer && npx vitest run tests/<file>`.</observation>
<parameter name="impact">Costs a worker a wrong diagnosis: the error names the main checkout and a temp file, so it reads as node_modules-depth breakage (ff61602d) rather than "you linked the wrong directory". The renderer half of the test matrix silently cannot run until it is fixed.</parameter>
<parameter name="recommendation">In a worktree: `ln -s /path/to/main/checkout/apps/desktop/src/renderer/node_modules apps/desktop/src/renderer/node_modules`. Verify with `ls src/renderer/node_modules/@vitejs` before trusting a renderer run.</parameter>
<parameter name="confidence">high</parameter>
<parameter name="related_paths">["apps/desktop/src/renderer/vitest.config.ts", "apps/desktop/package.json"]</parameter>
</invoke>
