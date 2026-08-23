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
last_tested: 2026-06-07
---

# Enable semantic search (local ONNX embeddings)

By default `rivet.context-recommend` matches lexically (shared words, tags,
paths). This procedure adds a local, offline embedding model so it also matches
on *meaning*. Everything runs on-box — no API keys, no network at query time.

> Run the shell commands below through your normal tools. Adjust paths/OS as
> noted. If you only need this occasionally, the **ollama** or **openai**
> backends (see the bottom) work with the default rivet binary and skip steps 1–3.

## Steps

1. **Build rivet with the ONNX tag.** The ONNX runtime dependency is opt-in, so
   it must be added and compiled in explicitly:
   ```bash
   go get github.com/yalue/onnxruntime_go
   CGO_ENABLED=1 go build -tags onnx -o "$(go env GOPATH)/bin/rivet" ./cmd/rivet
   ```

2. **Install the ONNX Runtime shared library.** Its version MUST match the
   binding's expected API version — binding v1.31.0 requires ONNX Runtime
   **1.26.0**. A mismatch shows "The requested API version [N] is not available".
   ```bash
   mkdir -p ~/.local/share/rivet/onnxruntime
   # Linux x64:
   curl -sSL https://github.com/microsoft/onnxruntime/releases/download/v1.26.0/onnxruntime-linux-x64-1.26.0.tgz \
     | tar xz -C /tmp && cp /tmp/onnxruntime-linux-x64-1.26.0/lib/libonnxruntime.so* ~/.local/share/rivet/onnxruntime/
   # macOS arm64: onnxruntime-osx-arm64-1.26.0.tgz  (lib is libonnxruntime.1.26.0.dylib)
   # macOS x64:   onnxruntime-osx-x86_64-1.26.0.tgz
   ```

3. **Download a sentence-embedding model** (model.onnx + vocab.txt). bge-small is
   a good default — small, fast, fully offline:
   ```bash
   M=~/.local/share/rivet/models/bge-small-en-v1.5; mkdir -p "$M"
   curl -sSL https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/onnx/model.onnx -o "$M/model.onnx"
   curl -sSL https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/vocab.txt   -o "$M/vocab.txt"
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

5. **Index the corpus** (embeds context + wiki + runbooks into `.rivet/embeddings/`):
   ```bash
   rivet context index
   ```
   Commit `.rivet/embeddings/` — it's keyed by model name, so teammates reuse
   the vectors and only need the model+runtime installed (not a re-index).

## Verification

A conceptual query with little word overlap should now carry a `semantic-match`
signal:
```bash
rivet context recommend "how do we stop unpaid accounts from logging in"
# each result's "signals:" line should include semantic-match
```

## Rollback

Unset `RIVET_EMBED_BACKEND` (or remove it from .mcp.json). Retrieval silently
falls back to lexical — the committed `.rivet/embeddings/` is harmless when
unused.

## Alternatives (no source build)

- **Ollama:** `RIVET_EMBED_BACKEND=ollama`, `ollama pull nomic-embed-text` — works
  with the default rivet binary; needs the ollama daemon.
- **OpenAI/compatible:** `RIVET_EMBED_BACKEND=openai` + `RIVET_EMBED_API_KEY` —
  trivial cost, but sends text off-box.
