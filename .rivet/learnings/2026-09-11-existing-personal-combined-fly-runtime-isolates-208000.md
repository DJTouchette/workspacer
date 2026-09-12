---
title: Existing personal combined Fly runtime isolates hub state and clears all app-secret environment variables
date: 2026-09-11
confidence: high
suggested_doc: fly-node-deploy
related_paths:
  - deploy/fly/combined/upgrade-supervisor.py
  - deploy/fly/combined/build-upgrade.sh
  - deploy/fly/inspect-machines.py
promoted: false
---

# Existing personal combined Fly runtime isolates hub state and clears all app-secret environment variables

## Observation
Live workspacer-node machine 1857645df24448 already runs a combined /opt/combined/supervisor.py from image 05c72b61f400f05f8375d72b8164eea0ba9bba799edc186710153d3f522844b0. Hub UID10002 uses /data/hub/home and separate provider/MCP tokens; worker UID10001 uses /data/home. Entry clears environment with env -i, and boot verifies migrated state manifest, Tailscale identity, owner modes, and forbids nodes.json. Replacing this with the generic combined entrypoint would discard the runtime's isolation and state homes. Fly public port8080 is a plain doorbell, and there were no allocated IPs when inspected; app UI remains on Tailscale.

## Recommendation
Upgrade this existing runtime by layering only changed binaries/web and a narrow supervisor patch. Preserve all state/identity checks. Pass only Fly identity metadata through env -i and inject a separately permissioned token file into the hub environment, never the worker's. Do not apply generic combined fly.toml to this existing machine.
