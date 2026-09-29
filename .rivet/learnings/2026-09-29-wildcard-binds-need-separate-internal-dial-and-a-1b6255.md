---
title: Wildcard binds need separate internal dial and advertised client addresses
date: 2026-09-29
suggested_doc: workspacer-serve-cli
related_paths:
  - Keep bind addresses in listener state; normalize only whereconstructing owned dial URLs. Do not replace concrete tailnet/LAN binds withloopback
  -  and do not use advertised addresses for child connections.
promoted: false
---

# Wildcard binds need separate internal dial and advertised client addresses

## Observation
Go hub busDialAddr maps IPv4 any to127.0.0.1 and IPv6 any to::1 while preserving concrete binds. Passing a raw wildcard into a plugin HUB_URL can connect by accident on Linux but fail Host validation and Windows dialing, silently losing provided capabilities. Rust now shares net_address::dial_addr for internal plugin/CLI/facade endpoints and treats user advertisement separately: interface enumeration,100.64/10 preference, first nonloopbackIPv4 fallback. Local HTTP maintenance probes bypass environment proxies; explicit concrete remote admin addresses retain their existing proxy behavior. IPv6 wildcard with noadvertisableIPv4 falls back to::1.
