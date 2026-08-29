---
title: A wholesale config path's else-branch DELETED the map: refuse a non-object, never coerce to {}
date: 2026-08-29
confidence: high
suggested_doc: config
related_paths:
  - services/hub/cmd/brain/config.go
  - services/hub/cmd/mcp/main.go
  - apps/desktop/src/main/services/configService.ts
  - contracts/wholesale-config-paths.json
promoted: false
---

# A wholesale config path's else-branch DELETED the map: refuse a non-object, never coerce to {}

## Observation
services/hub/cmd/brain/config.go applyWholesale ended `else { dst[leaf] = map[string]any{} }`. Because a wholesale path (ui.customThemes, claude.budgets, projects) is REPLACED rather than deep-merged, that branch did not degrade a malformed value — it deleted the user's whole map and returned the emptied config as a SUCCESSFUL save. writeConfigYAML backs up only an UNPARSEABLE file on read (.broken-<ts>), never a destructive successful write, so there is no automatic recovery path. Two non-obvious details: (1) on a config that is otherwise pristine the wipe is INVISIBLE, because the merged document comes out exactly equal to the shipped defaults and refuseWipeWithDefaults declines the write for an unrelated reason — a repro test only fails once one ordinary non-default setting is present; (2) the TS twin (configService.ts) took the SAME input and wrote it through verbatim (`dst[leaf] = src[leaf] ?? {}`), so the two config.yaml writers disagreed in opposite directions on the one input that loses data. The trigger was the MCP facade: save_config was registered with addObjectTool, whose SDK-inferred schema is exactly {"type":"object","additionalProperties":true} — verified by running — so a client that serialised its argument sent projects as a string and it was forwarded verbatim to the bus.

## Impact
Silent data loss on the hot path: every agent's config write answers in the brain, so an operator-tier agent could empty a user's project list (labels, colours, icons, favourite flags, delivery modes, yolo flags, worktreeSetup hooks) with a successful-looking save and nothing to restore from. This is the answer to the brief's open "the projects config map does not round-trip" item — its two candidate explanations were not opposites, one is the trigger and the other is the damage.

## Recommendation
Three layers now, and none is sufficient alone: applyWholesale returns errWholesaleNotAMap (propagated through saveLocked/save to a bus error), the TS twin throws WholesaleValueError, and cmd/mcp declares a real save_config InputSchema built from the wholesale path list plus a local handler check. contracts/wholesale-config-paths.json grew a `valueCases` block pinning both writers (and the facade's path list); the fixture came off both vocabulary guards' no-blocks exemption lists. null is refused with the rest — {} already spells "empty this map" in both languages. If you add a wholesale path, add it to the fixture's `paths` AND give it a valueCases row; the Go loader fails on an unexercised path.
