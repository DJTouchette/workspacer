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
last_tested: 2026-09-26
---

# Re-index embeddings after docs change

Run these commands from the Workspacer repository root. Semantic retrieval is
optional; `rivet doctor` reports whether an embedder is configured. These
instructions match Rivet v0.20.0 (`rivet --version`).

## Steps

1. Configure the same embedder for indexing and recommendation using
   [the setup runbook](setup-semantic-search.md). A cached document vector is
   not enough: recommendation still needs the embedder to encode each query.
2. Rebuild changed chunks:

   ```bash
   rivet context index
   ```

3. Check the printed document/chunk counts and exit status. With no backend,
   `index` fails with “no embedder configured.” A missing ONNX model, a binary
   built without the ONNX tag, or an unavailable HTTP backend can also fail
   indexing. Recommendation's lexical fallback does not make indexing succeed.
4. Keep `.rivet/embeddings/` local. Workspacer's `.gitignore` explicitly excludes
   it. Rivet's generic CLI prints “Commit that directory”; that advice does not
   apply to this repository. Do not force-add the cache.

## Verification

```bash
rivet context index
rivet context recommend "how do agents recover after a daemon disconnects"
```

The second index of unchanged docs with the same model should report zero
new/changed chunks. Inspect recommendation signals for `semantic-match`; merely
seeing the expected document is insufficient because lexical search may have
found it. A semantic signal proves the backend/index path is in use, not that
every recommendation is relevant.

The corpus includes context docs, wiki roots, and runbooks. Learning-log entries
are a separate unverified retrieval surface; indexing does not promote them.

## Changing models

Cache keys include the embedder ID and chunk content. In v0.20.0, HTTP identity
includes base URL and model name; ONNX identity uses the model directory's
**basename**, not a hash of model files. Replacing model files under the same
basename does not automatically invalidate vectors. Use a distinct versioned
model directory name, or move the old `.rivet/embeddings/` aside before rebuilding.
Changing only an HTTP API key does not change model identity.

## Rollback

Unset `RIVET_EMBED_BACKEND` in the shell and in the MCP server's launch
environment, then restart the MCP process if needed. Recommendation uses lexical
retrieval; leaving the ignored cache on disk is harmless.

## Last verification

2026-09-26: Linux x64, Rivet v0.20.0 built with ONNX binding v1.31.0,
ONNX Runtime 1.26.0, and the bge-small model. In an isolated copy of this
repository’s 45-doc corpus, indexing produced 227 vectors, a second index
added zero chunks, and the query above returned `semantic-match` signals.
The macOS alternatives and HTTP backends were not exercised in that check.
