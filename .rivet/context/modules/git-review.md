---
title: Host-side git service and review pane flows
tags: [git, review-pane, ipc, hub-bus, diff, containment]
related_paths:
  - "apps/desktop/src/main/services/gitService.ts"
  - "apps/desktop/src/renderer/src/lib/gitQueries.ts"
  - "apps/desktop/src/renderer/src/panes/ReviewPane.tsx"
  - "services/hub/cmd/brain/git.go"
owner: Damien Touchette
last_reviewed: 2026-09-16
---

# Host-side git service and review pane flows

The desktop host and headless brain expose `git.*` using the system `git`
binary. Authenticated agents have ambient host-path access, so callers may
select any repository the desktop user can access. Workspacer does not restrict
repository selection to live-agent cwd roots.

Once selected, semantic repository containment remains. `rev-parse
--show-toplevel` establishes the work-tree root, commands run from that root,
and caller pathspecs are anchored beneath it. The untracked-diff operand must
remain inside the selected work tree because `git diff --no-index` reads it as a
filesystem path. This is selected-object integrity, not a directory grant.

Key invariants:

- Resolve every cwd through `rev-parse --show-toplevel`.
- Treat `git diff --no-index` exit 1 with output as success.
- Keep porcelain and numstat rename parsers in sync.
- The router is single-owner; declare intentional desktop/headless overlaps.
- Renderer status types remain backward-compatible with older hosts.
- Remote view/triage/operator method policy remains separate from repo paths.

Run desktop git parser/service tests, renderer git query tests, headless git
tests (especially untracked selected-repo containment), overlap checks and the
review-pane browser flow for user-visible changes.
