---
title: Plugin installer state and settings must survive trusted code replacement
date: 2026-09-28
promoted: false
---

# Plugin installer state and settings must survive trusted code replacement

## Observation
Rust plugin tar.gz install stages and validates before touching the running incarnation, requires exact argv consent, and strips archive/build-provided loader markers before copying existing .bus-token/.settings.json/.disabled. Manager keeps credentials tracked across awaited registration/revocation so cancelled futures remain cleanable. HTTP plugin manifest projection is allowlisted; SDK settings injection requires host or the same plugin token and HTML-escapes JSON. Native supervisor owns polling log pipes rather than reader threads, so grandchildren retaining stdout cannot hold stop joins open.
