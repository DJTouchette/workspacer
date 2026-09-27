---
title: Embedded shutdown admission drain must be bounded for incomplete HTTP bodies
date: 2026-09-27
promoted: false
---

# Embedded shutdown admission drain must be bounded for incomplete HTTP bodies

## Observation
The spawn admission RwLock read guard covers JSON extraction, so a client holding an incomplete POST body can otherwise block shutdown forever. Bound the write-lock drain to two seconds and report failure while allowing the embedded owner to cancel its runtime and sweep PTY children. Lifecycle regression includes a stalled spawn body and restarting after shutdown; local host errors also preserve cleanup failures alongside the original failure.
