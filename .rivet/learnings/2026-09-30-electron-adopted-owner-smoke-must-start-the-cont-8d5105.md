---
title: Electron adopted-owner smoke must start the control plane independently
date: 2026-09-30
promoted: false
---

# Electron adopted-owner smoke must start the control plane independently

## Observation
Release36665384077 Linux job109728867003 passed the packaged owned-app case then failed its adopted fixture: --data-dir was placed before serve. Source review also found a circular readiness dependency from --external-claudemon before Electron starts that daemon. The fixture now starts serve --hub-only without borrowing an absent daemon, retaining the assertion that Electron owns/closes its daemon while borrowed hub/MCP survive. Packaged --help validates argv before startup. Actual corrected helper argv passed parser, authenticated hub/MCP readiness with absent daemon, and EOF exit0/closed ports in /tmp/workspacer-adopted-cli-proof.log. Helpers5/syntax/diff pass; complete packaged adoption awaits release rerun. No product ownership rule changed.
