---
title: Rust HTTP and push keep browser code while removing companion processes
date: 2026-09-28
promoted: false
---

# Rust HTTP and push keep browser code while removing companion processes

## Observation
Go hub HTTP routes beyond /bus and /health are plugin adapters, /remote, public /m/PWA/xterm assets, and optional /app static renderer. The Rust server embeds the same browser-only assets; no Node companion is needed. Host and browser Origin validation must use each accepted socket's actual local IP, not a wildcard listener's 0.0.0.0 address; explicit trusted-host names permit TLS reverse proxies without allowing DNS rebinding. Web Push uses pure Rust web-push-native payload encryption plus jwt-simple pure-rust. Its VAPID helper omits non-default endpoint ports from aud, so our adapter composes aud with URL origin and signs using the same library, covered by an offline encrypted-payload roundtrip and claim test. Notification tests use injected senders and never contact real push services.
