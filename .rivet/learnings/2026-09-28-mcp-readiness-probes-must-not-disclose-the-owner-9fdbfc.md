---
title: MCP readiness probes must not disclose the owner bearer
date: 2026-09-28
promoted: false
---

# MCP readiness probes must not disclose the owner bearer

## Observation
SessionFacade readiness checks the public nonsecret /health route without Authorization. Sending the owner bearer to /health before validating listener identity gives a wrong or reused listener the owner credential even when the subsequent readiness check rejects it. Rust health was already public; the temporary health_token plumbing was removed. Tests now assert both valid and mismatched listeners receive no Authorization header.
