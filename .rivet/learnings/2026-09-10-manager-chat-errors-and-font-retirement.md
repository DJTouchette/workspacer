---
title: Manager chat failure handling and font retirement need distinct ownership guards
confidence: high
suggested_doc: renderer-backend-seam
related_paths:
  - apps/desktop/src/renderer/src/panes/ClaudePane.tsx
  - apps/desktop/src/renderer/src/harness/firstUseHarness.tsx
  - apps/desktop/src/renderer/src/harness/fleetWorkflowHarness.tsx
  - apps/desktop/tests/e2e/managerHandoff.test.ts
promoted: false
---

Full CI 34444524581 exposed four uncaptured manager draft losses, two card
locator failures with a fabricated capture capability, an obsolete bootstrap
payload assertion, and a handoff page error. Only a host-issued request ID permits
the durable unknown-delivery branch. Without one, retain text/attachments, remove
the optimistic turn and report the original failure without a raw-input replay.
Generic fixture Proxy fallbacks must not invent managerRequest capabilities.

Hosted probe 34446934014 reproduced the handoff error in the real
@xterm/addon-web-fonts relayout: it checks _term before awaiting document.fonts.ready,
then reads _term.options after dispose cleared _term. ClaudePane already loads
fonts before opening and fences its own callback with disposed. Disable the
addon's separate automatic relayout; do not catch and hide its error or weaken
manager ownership assertions. The browser probe holds that real internal await
across replacement and retains the zero-page-error assertion.
