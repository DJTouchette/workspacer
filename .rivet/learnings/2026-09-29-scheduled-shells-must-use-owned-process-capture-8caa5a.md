---
title: Scheduled shells must use owned process capture too
date: 2026-09-29
promoted: false
---

# Scheduled shells must use owned process capture too

## Observation
The Rust jobs shell path originally spawned a bare Tokio child and detached a blocking combined-output reader. Unlike other owned commands it lacked Unix group cleanup and Windows suspended Job assignment, so scheduler cancellation could leave descendants/pipe readers alive. capture_combined now shares the existing Owner lifecycle, preserves stdout/stderr kernel-pipe ordering, releases parent Command write handles after spawn, and polls pipe reads without blocking tasks. Shell jobs now scrub ambient host-authority credentials consistently with other owned launches; this is an intentional difference from Go inherited environment.
