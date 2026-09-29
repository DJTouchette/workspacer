---
title: MCP manager request validation must precede provider forwarding
date: 2026-09-29
promoted: false
---

# MCP manager request validation must precede provider forwarding

## Observation
The Go manager request facade rejects empty requestId for get/resolve and empty taskId/cwd/reason for outcome acceptance even though the JSON schema only requires field presence. Rust schema validation alone accepted these and forwarded to the provider. A real MCP HTTP plus registered-provider fixture demonstrates the gap; workflows::prepare must reject before dispatch, while preserving legitimate ok:false conflict payloads as successful transport results.
