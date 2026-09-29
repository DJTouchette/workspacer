---
title: Windows per-child job ownership must precede user code
date: 2026-09-29
promoted: false
---

# Windows per-child job ownership must precede user code

## Observation
The old Go hub assigned itself to a kill-on-close job before spawning descendants. An embedded GUI must not assign itself, but replacing that with spawn-then-assign for individual commands leaves an immediate-fork gap. Microsoft Job Objects documentation specifies inheritance for children created after association: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects . Ordinary commands, plugin helpers and portable-pty must create suspended or assign at creation, establish the child job, then resume the original pinned process/thread. Existing descendant-after-assignment tests do not prove this boundary; jobobject_windows migration evidence is pending an actual immediate-fork runtime regression.
