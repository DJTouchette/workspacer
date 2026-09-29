---
title: Jobs output caps preserve byte offsets with lossy UTF-8 repair
date: 2026-09-29
promoted: false
---

# Jobs output caps preserve byte offsets with lossy UTF-8 repair

## Observation
Rust jobs head, tail and elide helpers in services/hub-rs/src/services/jobs.rs slice raw bytes then call String::from_utf8_lossy. They can insert replacement characters at split codepoints; they do not retain character boundaries. JOBS_MIGRATION.md was corrected during the manager review. Full library 311, jobs 8 and quiescence 3 passed on Linux; Windows quoted-shell runtime remains unproven by this run.
