---
title: Windows Git evidence needs inode-verified alias scope for 8.3 temp paths
date: 2026-09-13
confidence: high
suggested_doc: git-review
related_paths:
  - apps/desktop/src/main/services/intentEvidenceCapture.ts
  - apps/desktop/src/main/services/intentEvidenceCapture.test.ts
  - apps/desktop/src/main/services/intentWindowsFiles.test.ts
promoted: false
---

# Windows Git evidence needs inode-verified alias scope for 8.3 temp paths

## Observation
Actual Windows runner diagnostics confirm Node realpath preserves C:\Users\RUNNER~1 while Git --show-toplevel returns C:/Users/runneradmin. Porcelain reports the modified file, but both byte-exact confinement checks fail and discard it; secret=false. Capture now derives the execution subtree from Git --show-prefix, accepts that comparison spelling only when bigint dev+ino match the original host cwd, rejects unavailable alias identity, and rechecks both directories after capture. Confinement and secret checks retain the original spelling too, so a config root using an 8.3 alias is not bypassed. No global case folding or guard relaxation.
