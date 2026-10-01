---
title: Native child rows retain bounded telemetry and separate agent identities
date: 2026-10-01
promoted: false
---

# Native child rows retain bounded telemetry and separate agent identities

## Observation
Provider-neutral daemon SubagentInfo emits camelCase startedAt/completedAt integer epoch milliseconds and toolUseId, model, lastToolName/lastToolSummary; Electron enriches tokens/costUSD/toolCalls. Native bounded_inventory formerly copied only strings, silently discarding numeric telemetry. New child projection copies only scalar telemetry, preserves omitted fields on retained inventory IDs, and treats supplied arrays as authoritative membership. Native provider agent IDs never become Workspacer session links; exact toolUseId anchors may contain multiple children, Workspacer receipts anchor session IDs separately, and unanchored fleet children derive from parentSessionId. Legacy Claude order fallback requires complete one-to-one available rows with chronological timestamp intervals; explicit missing anchors remain unanchored. Context tokens must not be presented as total consumption.
