---
title: MCP optional workflow fields and tool failure envelopes are wire contracts
date: 2026-09-29
promoted: false
---

# MCP optional workflow fields and tool failure envelopes are wire contracts

## Observation
The preserved desktop dispatch-chain suite exposed that a missing first-step afterDispatchId must remain absent: explicit null fails the TS exact next-step guard. Rust MCP now omits absent optional spawn fields and reports tool argument/domain validation failures as isError content, while unknown/unavailable tool and transport authentication failures remain protocol errors.
