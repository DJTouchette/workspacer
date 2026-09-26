---
title: Enable semantic search (local ONNX embeddings)
triggers:
  - enable semantic search
  - set up embeddings
  - semantic search not working
  - set up onnx
  - context recommend only lexical
  - turn on embeddings
severity: low
owner: rivet
last_tested: 2026-09-26
---

# Enable semantic search (local ONNX embeddings)

By default `rivet.context-recommend` matches lexically (shared words, tags,
paths). This procedure adds a local, offline embedding model so it also matches
on *meaning*. Everything runs on-box — no API keys, no network at query time.

> Run the shell commands below through your normal tools. Adjust paths/OS as
> noted. If you only need this occasionally, the **ollama** or **openai**
> backends (see the bottom) work with the default rivet binary and skip steps 1–3.

## Steps

1. **Build Rivet with the ONNX tag in a separate Rivet source checkout.**
   Workspacer does not contain the Rivet CLI source. Use a writable checkout of Rivet
   v0.20.0 for this recipe; run the following from that checkout, not Workspacer.
   Pin the binding so it stays matched to the runtime in step 2:
   ```bash
   go get github.com/yalue/onnxruntime_go@v1.31.0
   CGO_ENABLED=1 go build -tags onnx -o "$(go env GOPATH)/bin/rivet" ./cmd/rivet
   ```

2. **Install the ONNX Runtime shared library.** Its version MUST match the
   binding's expected API version — [binding v1.31.0](https://github.com/yalue/onnxruntime_go/blob/v1.31.0/README.md)
   uses the ONNX Runtime **1.26.0** C API headers. This recipe keeps the runtime
   at that matching version; do not upgrade one side independently. A mismatch shows "The requested API version [N] is not available".
   ```bash
   mkdir -p ~/.local/share/rivet/onnxruntime
   # Linux x64:
   curl -fLsS https://github.com/microsoft/onnxruntime/releases/download/v1.26.0/onnxruntime-linux-x64-1.26.0.tgz \
     | tar xz -C /tmp && cp /tmp/onnxruntime-linux-x64-1.26.0/lib/libonnxruntime.so* ~/.local/share/rivet/onnxruntime/
   # macOS arm64: onnxruntime-osx-arm64-1.26.0.tgz  (lib is libonnxruntime.1.26.0.dylib)
   # macOS x64:   onnxruntime-osx-x86_64-1.26.0.tgz
   ```

3. **Download a sentence-embedding model** (model.onnx + vocab.txt). bge-small is
   a good default — small, fast, fully offline:
   ```bash
   rivet_model_dir=~/.local/share/rivet/models/bge-small-en-v1.5
   mkdir -p "$rivet_model_dir"
   curl -fLsS https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/onnx/model.onnx -o "$rivet_model_dir/model.onnx"
   curl -fLsS https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/vocab.txt   -o "$rivet_model_dir/vocab.txt"
   ```

4. **Point rivet at them** via three env vars (paths are machine-specific):
   ```bash
   export RIVET_EMBED_BACKEND=onnx
   export RIVET_EMBED_MODEL=~/.local/share/rivet/models/bge-small-en-v1.5
   export RIVET_EMBED_ORT_LIB=~/.local/share/rivet/onnxruntime/libonnxruntime.so.1.26.0
   ```
   For the MCP server (Claude Code), add the same three to the `rivet` entry's
   `"env": {}` block in `.mcp.json`. Don't commit machine-specific absolute
   paths to a shared .mcp.json — set them per-developer.

5. **Return to the Workspacer repository root and index the corpus** (context,
   wiki, and runbooks into `.rivet/embeddings/`):
   ```bash
   rivet context index
   ```
   Keep `.rivet/embeddings/` local: Workspacer ignores this generated cache.
   The generic CLI suggestion to commit it does not apply here. Each developer
   indexes locally; see [re-indexing](reindex-embeddings.md) for model identity
   and invalidation rules.

## Verification

Indexing must exit successfully. A missing model/runtime or a non-ONNX build
is an indexing error even though recommendation can fall back to lexical.
The MCP server must inherit the same environment; exporting variables after
it starts does not reconfigure the already-running process.


A conceptual query with little word overlap should now carry a `semantic-match`
signal:
```bash
rivet context recommend "how do agents recover after a daemon disconnects"
# inspect result signals for semantic-match; lexical matches alone do not prove setup
```

## Rollback

Unset `RIVET_EMBED_BACKEND` (or remove it from .mcp.json). Retrieval silently
falls back to lexical — the ignored `.rivet/embeddings/` is harmless when
unused.

## Alternatives (no source build)

- **Ollama:** `RIVET_EMBED_BACKEND=ollama`, `ollama pull nomic-embed-text` — works
  with the default rivet binary; needs the ollama daemon.
- **OpenAI/compatible:** `RIVET_EMBED_BACKEND=openai` + `RIVET_EMBED_API_KEY` —
  sends indexed document text and recommendation queries to the configured
  endpoint. `RIVET_EMBED_MODEL` selects the model and `RIVET_EMBED_BASE_URL`
  selects the API base. Keep credentials in the launch environment.

## Last verification

2026-09-26: Linux x64, Rivet v0.20.0 built with ONNX binding v1.31.0,
ONNX Runtime 1.26.0, and the bge-small model. In an isolated copy of this
repository’s 45-doc corpus, indexing produced 227 vectors, a second index
added zero chunks, and the query above returned `semantic-match` signals.
The macOS alternatives and HTTP backends were not exercised in that check.
