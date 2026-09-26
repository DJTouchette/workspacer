# Workspacer project knowledge

Rivet supplies project context to agents working in this repository. Start with
`rivet context recommend "your task"`, then `rivet context show <name>` and the
referenced implementation. Context documents explain current behavior;
learning-log entries are observations that still need verification.

- `context/domains`: system areas and their contracts.
- `context/modules`: specific subsystems and maintenance guidance.
- `context/paradigms`: patterns shared across subsystems.
- `runbooks`: operational procedures, with explicit verification steps.
- `learnings`: captured findings; `archive` retains historical entries.
- `recon` and `embeddings`: ignored generated caches, rebuilt locally.

## Updating documentation

Check behavior against executable source and relevant tests. Historical source
comments can also be stale. Keep useful failure modes and invariants, but replace
contradictory historical notes instead of appending a new rule beside the old one.
Use repository-root paths in `related_paths` and source-code references; Markdown
links are relative to the document containing them.

Add retrieval tags for the terms a future task will use. Change `last_reviewed`
only after reviewing the whole guide; a path or tag correction is not a full
review. For runbooks, record the platform, versions, actual results, and any
untested alternatives alongside `last_tested`.

From the repository root:

```bash
rivet doctor
rivet context lint
python3 scripts/check-rivet-docs.py
git diff --check
```

These checks cover environment, metadata/retrieval issues, and local references.
They do not establish semantic accuracy or test all documented behavior. Run
the implementation checks relevant to the claims changed, and distinguish a
passing test from a skipped test or a filter that selected nothing.

## Discovery and retrieval

`AGENTS.md` is a tracked symlink to `CLAUDE.md`. Refresh the shared Rivet-managed
section with `rivet sync --provider claude`; it preserves the hand-authored text
after the markers. Do not edit inside the markers. Using `--provider both`
writes the same target twice and leaves the Codex-specific version in that
shared file, removing the Claude project-agent guidance.

Lexical retrieval works without embedding configuration. Semantic retrieval is
optional; follow [setup](runbooks/setup-semantic-search.md) and
[re-indexing](runbooks/reindex-embeddings.md). Workspacer keeps embedding caches
ignored even when Rivet's generic command output suggests committing them.

The [audit ledger](../docs/reviews/workspacer-rivet-audit.md) records review
coverage and evidence. A clean lint result must not be represented as completion
of an unfinished source review.
