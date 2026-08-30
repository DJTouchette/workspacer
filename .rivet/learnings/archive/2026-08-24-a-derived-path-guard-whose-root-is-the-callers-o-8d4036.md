---
title: A derived-path guard whose ROOT is the caller's own cwd stops narrowing when the cwd is $HOME
date: 2026-08-24
confidence: high
suggested_doc: webview-security-hardening
related_paths:
  - apps/desktop/src/main/services/hubCapabilities.ts
  - services/hub/cmd/brain/library.go
  - contracts/path-containment-cases.json
promoted: true
promoted_to: webview-security-hardening
---

# A derived-path guard whose ROOT is the caller's own cwd stops narrowing when the cwd is $HOME

## Observation
library.list/save/remove guard the caller's `cwd` and then guard the DERIVED item path (<cwd>/.workspacer/library/<slug>.md, <cwd>/.claude/skills/<id>/SKILL.md) against `libraryItemRoots(canonicalCwd)` = [<configDir>/library, canonicalCwd]. That second guard is only as narrow as the cwd the caller named — and library.list checks its cwd against the BROWSE roots (workspace + the whole home tree), because the New Agent dialog lists a directory no agent runs in yet. So a caller may name $HOME, the item roots become the whole home tree, and a `$HOME/.workspacer/library/a.md -> $HOME/.ssh/id_rsa` symlink canonicalizes inside the root, passes, and comes back as an item BODY — while fs.read of the identical path is refused for the same caller. The brain (cmd/brain/library.go) closed this with a second, LEXICAL requirement (libraryItemDirs + containsPath) that the RESOLVED file sit in a directory a library item actually lives in; hubCapabilities.ts shipped without it, so the two providers disagreed about the same call and the wide one is what DELEGATE_CATALOG_TO_BRAIN=false puts back on the bus. Fixed 2026-08-24 (commit 3158c6f2) and pinned by a new `libraryItemDirs` block in contracts/path-containment-cases.json with two loaders. Note the second gate must stay LEXICAL: canonicalizing the two cwd-derived directories resolves the very link it exists to see.

## Impact
The general lesson beyond library.*: when a derived-path guard's ROOT is itself derived from the caller's directory, the guard is self-fulfilling for any method whose cwd may be a wide root. Ask what the WIDEST cwd that method accepts is, not what a typical one is.

## Recommendation
Any new capability that composes a provider-chosen path under a caller-supplied directory needs BOTH halves: containment against roots, and a requirement about where the resolved file may live. Add its cases to contracts/path-containment-cases.json (`methods.derivedRootSet` for the first, the `libraryItemDirs` block's pattern for the second) rather than writing one-off tests.
