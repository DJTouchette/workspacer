---
title: Loopback Python fixtures must avoid reverse DNS at HTTP server bind
date: 2026-09-30
promoted: false
---

# Loopback Python fixtures must avoid reverse DNS at HTTP server bind

## Observation
Mac run36663252203 pinpointed local_spawn waiting for a Codex child after the provider had ended. Diagnostic36663801199 showed interpreter/before-bind only, then ReadinessFailed and owned cleanup. Standard HTTPServer.server_bind calls getfqdn after raw TCP bind. Replacing only that hostname lookup with fixed localhost/actual port, guarded against DNS calls, passed all three Mac attempts in run36664199884. Production Rust behavior was unchanged. Main retains the narrow fixture correction, named wait phases and rejection of ended-as-idle readiness; diagnostic workflow/subscriber code stays off main.
