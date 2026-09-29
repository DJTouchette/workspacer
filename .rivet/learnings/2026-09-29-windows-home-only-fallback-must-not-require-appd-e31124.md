---
title: Windows home-only fallback must not require AppData directories
date: 2026-09-29
suggested_doc: workspacer-serve-cli
promoted: false
---

# Windows home-only fallback must not require AppData directories

## Observation
The Windows directories BaseDirs constructor requires profile, roaming AppData and local AppData together. With USERPROFILE removed or empty, an AppData lookup can fail even when the caller only needs a home. CLI fallback now uses UserDirs, which requires only profile; optional folder failures do not erase home. The subprocess fixture retains the Go servedb contract: absolute OS home or explicit lookup failure, never relative guessing; explicit --home-dir must still succeed without creating files.
