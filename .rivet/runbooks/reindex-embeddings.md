---
title: Re-index embeddings after docs change
triggers:
  - stale embeddings
  - recommend missing new docs
  - re-index embeddings
  - semantic results out of date
  - added context docs but recommend ignores them
severity: low
owner: rivet
last_tested: 2026-06-07
---

# Re-index embeddings after docs change

Semantic recommend reads precomputed vectors from `.rivet/embeddings/`. New or
edited context/wiki/runbook docs aren't matched semantically until they're
embedded.

## Steps

1. Make sure the embedder env is set (see the "Enable semantic search" runbook
   if `rivet context recommend` warns that semantic is disabled).
2. Re-index — only new or changed chunks are embedded, so this is cheap:
   ```bash
   rivet context index
   ```
3. Commit the updated `.rivet/embeddings/` (deterministic; minimal diff).

## Verification

```bash
rivet context recommend "<a topic from a doc you just added>"
# the new doc should appear with a semantic-match signal
```

## Notes

Switching embedding models invalidates the cache automatically (it's keyed by
model). A missing model at index time just leaves those vectors unset — rivet
warns rather than failing.
