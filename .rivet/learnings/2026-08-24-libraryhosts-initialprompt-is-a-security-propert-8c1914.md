---
title: LibraryHost's initialPrompt is a security property: never auto-send text that can come off disk
date: 2026-08-24
confidence: high
suggested_doc: agent-spawn
promoted: false
---

# LibraryHost's initialPrompt is a security property: never auto-send text that can come off disk

## Observation
Library items have a PROJECT scope stored at `<cwd>/.workspacer/library/*.md` — per repo, committable — and both LibraryPane and CommandPalette render a Dispatch button on every one. What keeps a repo-shipped prompt from RUNNING on one click is a single line: LibraryHost.tsx's spawn branch passes `initialPrompt` (composer pre-fill) rather than `kickoffMessage` (auto-send). The app's own auto-send call sites (spawnGuide, spawnFleetManager) are safe for a reason that does not transfer: they send text the APP composed in code around a question the USER typed.</observation>
<parameter name="impact">Swapping that one field to kickoffMessage would let a cloned repo run a prompt of its choosing with no read step. It was documented nowhere until 2026-08-24.</impact>
<parameter name="recommendation">The rule: never auto-send text that can come off disk; auto-send is only for text the app owns in code. It is now pinned by a comment at the call site, by LibraryHost's Props type (which has no kickoffMessage field), and by tests/libraryHostAutoSend.test.tsx. The same rule governs lib/draftAgent.ts, whose bus event carries a brief ID rather than text so free text cannot enter that path at all.</recommendation>
<parameter name="related_paths">["apps/desktop/src/renderer/src/components/LibraryHost.tsx", "apps/desktop/src/renderer/src/lib/draftAgent.ts", "apps/desktop/src/renderer/tests/libraryHostAutoSend.test.tsx"]
