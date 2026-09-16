---
title: Headless agent tools require owned facade
date: 2026-09-16
promoted: false
---

# Headless agent tools require owned facade

## Observation
The brain must never infer 127.0.0.1:7897: workspacer serve historically supervised only claudemon and hub, so an inferred URL produced a green spawn with dead tools. The launcher that owns cmd/mcp now passes the URL hub-to-brain only after reserving the port and waits for /health hubConnected:true; desktop adopts a healthy external facade instead of killing its listener.
