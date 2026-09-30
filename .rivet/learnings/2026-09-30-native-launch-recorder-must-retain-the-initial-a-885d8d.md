---
title: Native launch recorder must retain the initial argv across provider fallback
date: 2026-09-30
promoted: false
---

# Native launch recorder must retain the initial argv across provider fallback

## Observation
CI ab3d40f1 claudemon job109704858726 failed the Codex native launch golden case with the fallback model override argv replacing app-server/--listen. The recorder already renamed complete files, so this was not a partial-write race: its deliberate exit1 allowed a later fallback invocation to overwrite the first complete capture. Test-only publication now uses a per-PID temporary file and no-clobber hardlink, with a deterministic two-invocation control. Production provider fallback remains unchanged; the new two-invocation control and original native golden each passed, and five further golden runs each executed and passed one test. The same compiled claudemon library passed905 tests with4 existing ignored checks (/tmp/workspacer-command-full-lib.log). Witness suggested only cross-language co-change tests, so the actual owning Rust fixtures supplied validation.
