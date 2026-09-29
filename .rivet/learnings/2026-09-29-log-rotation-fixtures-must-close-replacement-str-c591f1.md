---
title: Log rotation fixtures must close replacement streams before cleanup
date: 2026-09-29
promoted: false
---

# Log rotation fixtures must close replacement streams before cleanup

## Observation
The real logFile rotation tests deleted their scratch directory while asynchronous createWriteStream/rotation callbacks were still active. Full desktop tests exposed ENOENT opening the next log after teardown. Track every created WriteStream and await close before removing the directory; the tracked array also captures replacement streams created by close callbacks. Restore console passthrough first so teardown cannot enqueue more logging.
