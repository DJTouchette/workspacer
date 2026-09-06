---
title: HTML card diffs must bypass ReviewPane fuzzy matching and pin byte reads
date: 2026-09-05
confidence: high
suggested_doc: git-review
related_paths:
  - apps/desktop/src/main/services/gitService.ts
  - apps/desktop/src/main/services/htmlCardPaths.ts
  - apps/desktop/tests/e2e/htmlCard.test.ts
promoted: false
---

# HTML card diffs must bypass ReviewPane fuzzy matching and pin byte reads

## Observation
ReviewPane's ordinary open-file flow resolves by suffix or basename. HTML cards now use a separate owner-derived readHtmlCardDiff IPC and an inline DiffView so exact canonical paths and pinned-descriptor text bytes stay together. Chromium request events can include CSP-blocked CSS/image requests; count route interception separately and require a CSP failure (`csp` or `net::ERR_BLOCKED_BY_CSP`, depending on browser tooling) before claiming no egress.
