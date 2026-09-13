---
title: Immutable artifact files require pinned parent directories
date: 2026-09-12
confidence: high
suggested_doc: git-review
related_paths:
  - apps/desktop/src/main/services/intentArtifactFiles*.ts
  - apps/desktop/src/main/services/intentArtifactStore*.ts
  - apps/desktop/src/main/services/intentEvidenceCapture*.ts
promoted: false
---

# Immutable artifact files require pinned parent directories

## Observation
Leaf O_NOFOLLOW or open('wx') does not stop a symlinked artifact directory, and realpath-then-mkdir can be redirected between checks. intentArtifactFiles traverses each absolute path component via a pinned directory FD, refuses symlinks with O_DIRECTORY|O_NOFOLLOW, creates missing components under the pinned parent, and keeps the final directory handle until the file and SQLite transaction finish. Git dirty evidence also uses isolated temporary Git metadata with frozen HEAD/copied bounded index and disabled inherited/global config: enumerating filter.* once cannot prevent a new clean/process driver added before a later Git diff.

## Recommendation
Use openIntentArtifactDirectory/openIntentArtifactFile for host-owned artifact or knowledge byte writes and bounded reads. The current helper fails closed outside Linux; add an equivalent directory-relative OS primitive before enabling writes on other platforms. Preserve the late-filter-injection and directory-swap regression tests.
