---
title: Paired workflow admission must book a local proxy before the remote process
date: 2026-09-28
promoted: false
---

# Paired workflow admission must book a local proxy before the remote process

## Observation
The ordinary qualified hub spawn and workers-only paired workflow route have different ownership contracts. Paired settings live in remote-server.json mode workers; source identity must change when endpoint or credential changes. The Rust paired route keeps the local task cwd for TaskStore admission, treats remoteCwd and prepared execution cwd as opaque strings, books a paired:<uuid> attempt before forwarding agents.spawn, and sends only rendered message/model/tool fields to the peer. That proxy stays in a separate observation map so engine mutations and local ownership lookup cannot mistake it for a local session. Terminal schema and escalation validation occurs again at the origin from final reply text; peer result/review/grant fields never establish local workflow evidence. Unknown spawn acknowledgement retains the booked attempt and prevents repeating a workflow step.
