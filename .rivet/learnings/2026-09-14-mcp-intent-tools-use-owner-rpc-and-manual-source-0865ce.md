---
title: MCP intent tools use owner RPC and manual sources for collected context
date: 2026-09-14
promoted: false
---

# MCP intent tools use owner RPC and manual sources for collected context

## Observation
The MCP intents group exposes create_intent, list_intents, get_intent, update_intent and add_intent_context through the closed action vocabulary of desktop.intentWorkspaceRequest. This RPC requires authenticated host authority on the bus, so facade integration tests must use a synthetic SetToken owner connection like the desktop facade launcher does; anonymous or scoped operator bus connections cannot call it. Facade tool availability remains operator-only via b.allowed, without broadening the desktop RPC. Create fixes status to draft; update merges omitted fields and forwards revision plus updatedAt guards while preserving status. Collected context uses manual sources, which now permit an empty URL for local notes; external providers still require valid URLs. Source IDs make repeated identical context attachment idempotent, while create remains non-idempotent and help instructs listing after ambiguous failures.
