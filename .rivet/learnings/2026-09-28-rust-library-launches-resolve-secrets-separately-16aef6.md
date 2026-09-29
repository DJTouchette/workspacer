---
title: Rust library launches resolve secrets separately from public library results
date: 2026-09-28
promoted: false
---

# Rust library launches resolve secrets separately from public library results

## Observation
Rust Library::list/save mask MCP env and headers using __WKS_SECRET__; saves restore unchanged placeholders from the guarded on-disk item. SessionFacade must use Library::selected_mcp rather than a public listing, otherwise providers receive redaction placeholders. Project definitions override global IDs; Claude skill IDs retain actual basenames and unknown frontmatter. Dispatch placeholder parsing now executes contracts/dispatch-template-params-cases.json, including ECMAScript whitespace cases.
