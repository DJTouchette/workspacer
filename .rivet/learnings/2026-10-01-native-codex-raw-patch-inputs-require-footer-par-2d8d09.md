---
title: Native Codex raw patch inputs require footer parity
date: 2026-10-01
promoted: false
---

# Native Codex raw patch inputs require footer parity

## Observation
Codex apply_patch tool inputs can be a raw JSON string or an object with input containing the raw patch. The rich tool preview already handles both, but file summaries must accept them too so chat footer and historical summaries retain edited files. Workspacer MCP receipt isError must refuse child-session linking even when the outer normalized tool result omitted is_error.
