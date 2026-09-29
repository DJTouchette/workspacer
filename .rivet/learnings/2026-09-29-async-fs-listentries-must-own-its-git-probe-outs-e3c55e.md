---
title: Async fs listEntries must own its Git probe outside spawn_blocking
date: 2026-09-29
suggested_doc: hub-process-supervision
related_paths:
  - Do not wrap an owned asynchronous process supervisor in spawn_blocking inside a cancellable Hub handler; cancellation cannot stop the detached blocking closure. Test hub shutdown with a delayed real descendant and scrub synthetic host-token overlays without mutating global environment.
promoted: false
---

# Async fs listEntries must own its Git probe outside spawn_blocking

## Observation
Putting files.call entirely inside spawn_blocking detached its synchronous Git check-ignore runtime from cancellation of the Hub handler. Hub shutdown could finish its2s grace while that helper continued up to5s and descendants could still perform delayed work. fs.listEntries now keeps only directory enumeration and result formatting in spawn_blocking; the awaitable owned_process::capture_input future belongs directly to the registered handler. The public synchronous files.call retains its bounded separate-thread runtime for nonasync callers. Both paths share entry shaping, fixed Git argv, NUL-delimited names, status0/1 handling and fallback behavior.
