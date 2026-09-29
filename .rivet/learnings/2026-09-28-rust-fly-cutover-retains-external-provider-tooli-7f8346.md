---
title: Rust Fly cutover retains external provider tooling and isolated hub credentials
date: 2026-09-28
promoted: false
---

# Rust Fly cutover retains external provider tooling and isolated hub credentials

## Observation
Rust deployment preview preserves Node22/npm and the existing Claude Code installation for third-party provider/plugin runtimes; only our Go backend binaries and desktop-host.cjs companion are removed. Split role images reuse mounted HOME, bootstrap state-loss guards and Tailscale supervision while starting one owned worker graph. The actual protected combined supervisor code was inspected locally at /opt/combined/supervisor.py without reading live state: hub UID10002 holds Fly/network credentials and worker UID10001 has a pristine environment. Its reviewed AST fixture permits only narrow hub/worker spawn-block replacement and refuses unknown policy, requiring preexisting separate operator facade authority and worker local identity. Generic combined single-user startup must not replace that isolated topology. Existing artifact fetching validates release stamps and logs, but does not authenticate, an archive checksum.
