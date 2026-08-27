---
title: The Fly node base image installs ONLY Claude Code — codex/opencode/pi are NOT in it
date: 2026-08-26
confidence: high
suggested_doc: remote-mobile
related_paths:
  - deploy/fly/node/Dockerfile
  - deploy/fly/node/bootstrap.sh
  - deploy/fly/node/example.Dockerfile
  - deploy/fly/node/BASE_IMAGE.md
  - deploy/fly/node/verify-image.sh
promoted: false
---

# The Fly node base image installs ONLY Claude Code — codex/opencode/pi are NOT in it

## Observation
deploy/fly/node/Dockerfile:178 is the only agent-CLI install in the node image (`npm install -g @anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}`). There is no codex, opencode, or pi install anywhere in deploy/. Verified three independent ways: (1) `grep -rniE "npm install -g|cargo install|@openai/codex|opencode" deploy/` returns only the claude line; (2) verify-image.sh asserts nothing about other agent binaries; (3) bootstrap.sh:148 merely PRE-CREATES `$WKS_HOME/.codex` with the comment "codex OAuth + config.toml + skills, **if the codex provider is used**" — a volume-persistence provision, not evidence of an install. The base is deliberately Claude-only; example.Dockerfile + BASE_IMAGE.md establish that extra toolchains belong in a downstream project layer, installed to /usr/local (anything under $HOME is shadowed the instant the Fly volume mounts at boot).</observation>
<parameter name="impact">A widely-held belief in session memory and in scout-task premises is that "codex-cli was added to wks-node". It was not. Any plan that says "add provider X to the node image the way codex was added" is planning against a change that does not exist in this repo, and will size the work wrongly — the real shape is a downstream image layer plus a bootstrap.sh persistent-dir entry, not a base Dockerfile edit.

## Recommendation
When adding a new agent provider to headless/Fly nodes: (1) add the persistent dotfile dir to bootstrap.sh's dir list next to .claude/.codex; (2) install the CLI in a DOWNSTREAM Dockerfile layer with the base's existing /usr/local npm prefix, never under $HOME; (3) re-run verify-image.sh, which is mandatory and must stay last. Do not edit deploy/fly/node/Dockerfile to add project-specific agent CLIs.
