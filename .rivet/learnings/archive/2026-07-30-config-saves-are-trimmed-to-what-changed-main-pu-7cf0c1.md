---
title: Config saves are trimmed to what changed; main pushes config changes
date: 2026-07-30
promoted: true
---

# Config saves are trimmed to what changed; main pushes config changes

## Observation
ConfigContext.save now runs the partial through minimalConfigPatch(snapshot, partial) before the IPC. Callers spread whole subtrees ({ui: {...config.ui, x}}) from a snapshot that can be minutes old, and configService.saveConfig replaces ui.customThemes and claude.budgets WHOLESALE — so a sidebar drag could delete a theme created on the phone client, and a keep-warm toggle could wipe claude.seenModels an agent had just written. deepMerge already handled leaf-only partials, so the spreads were never needed; trimming at the seam fixes all 43 call sites without touching one. TRAP I hit building it: the diff must NOT recurse into the wholesale paths — recursing turns a deletion into an empty patch, so the delete silently never happens. WHOLESALE_PATHS in lib/configPatch.ts is a twin of the explicit handling in configService.saveConfig; keep them in step. Second half: configService.onChange + an fs.watch on the config DIRECTORY (not the file — atomic writes replace the inode and kill a file watch), mtime-gated so our own writes don't echo, pushed to the renderer over the new config:changed channel. That is what catches writes by main itself and by the Go brain serving web/phone clients. Adding the channel also required stubbing configService.onChange in src/main/ipc.test.ts (subscribed at registration time) and triaging onConfigChanged in backendParity.test.ts.

## Disposition
Folded into .rivet/context/domains/config.md (minimalConfigPatch + config:changed push note).
