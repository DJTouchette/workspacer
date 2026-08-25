---
title: save_config deep-merges under a lock with compare-and-swap — it does not clobber
date: 2026-08-24
confidence: high
suggested_doc: config
promoted: false
---

# save_config deep-merges under a lock with compare-and-swap — it does not clobber

## Observation
The facade's `save_config` tool → `config.save` → `configService.saveConfig` is a read-modify-write under a cross-process O_EXCL lockfile: it re-reads config.yaml from disk UNCONDITIONALLY, deepMerges the caller's partial onto it, compare-and-swaps on `mtimeMs:size` immediately before publishing (retrying up to 5 times if a non-lock-participating writer beat it), then writes atomically. The Go twin in cmd/brain/config.go mirrors all of it. A failed lock is a REFUSED save, never a write-anyway. Only three paths are replaced wholesale rather than merged — `ui.customThemes`, `claude.budgets`, `projects` — and only when the caller sends them; they are pinned across both writers by contracts/wholesale-config-paths.json. Bus callers (which an agent is) additionally have HOST_TRUSTED_SECTIONS/PATHS stripped before the merge.</observation>
<parameter name="impact">The practical answer to "why not just let an agent edit the JSON": for ordinary settings we already can, and safely. The one thing an agent must be told is the wholesale rule — patching ui.customThemes without resending the whole map deletes every other custom theme, and nothing warns it.</impact>
<parameter name="recommendation">Do not build clobber-protection machinery around agent-driven config edits; it already exists and predates this question. Do put the wholesale-map warning in any app-owned prompt that lets an agent touch ui.customThemes (lib/draftAgent.ts's appearance brief does, pinned by tests/draftWithAgent.test.tsx).</recommendation>
<parameter name="related_paths">["apps/desktop/src/main/services/configService.ts", "services/hub/cmd/brain/config.go", "apps/desktop/src/main/shared/configWholesale.ts"]
