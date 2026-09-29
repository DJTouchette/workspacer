---
title: Embedded Rust MCP must share the socket-aware proxy policy
date: 2026-09-28
promoted: false
---

# Embedded Rust MCP must share the socket-aware proxy policy

## Observation
The RMCP transport has its own static loopback Host allowlist. Even after the outer Workspacer actual-socket/trusted-host and Origin checks approve an explicitly configured reverse proxy, RMCP independently returned403. Rust MCP now disables that redundant SDK Host list while keeping the shared outer host guard and per-request Host/Origin+credential gate. The real trusted-proxy MCP fixture exposed this; the fresh MCP7 suite passed after the fix. Electron Rust preview adopts only the migration service with embedded transport and exact hubBusUrl; it never launches a second standalone facade.
