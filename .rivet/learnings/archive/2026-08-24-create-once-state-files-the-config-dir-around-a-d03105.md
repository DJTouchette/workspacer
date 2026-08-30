---
title: Create-once state files: the config dir around a missing file is the first-run vs state-loss discriminator
date: 2026-08-24
promoted: true
promoted_to: config
---

# Create-once state files: the config dir around a missing file is the first-run vs state-loss discriminator

## Observation
Three loaders were shaped "read; on ENOENT, create a new one", which is correct on a first run and wrong on every later one because a recreated credential is a DIFFERENT credential: `<config>/workspacer/remote-token` (cmd/workspacer/token.go), `<config>/workspacer-hub/vapid.json` (internal/push), `<config>/workspacer/config.yaml` (cmd/brain, loadFromDisk's first-run arm).

internal/statelost (Go) + apps/desktop/src/main/lib/stateLoss.ts (TS twin) now answer the question those loaders could not: is the directory around the missing file empty (nobody has ever run here) or does it still hold the rest of the state (something took this one file away)? `Suspected(dir, name)` = the dir is readable and holds ≥1 entry other than `name`.

The three got DIFFERENT answers on purpose, keyed to what the process is:
- remote-token → `workspacer serve` REFUSES to start (foreground CLI, exit is the loudest signal, no useful work a mis-identified node can do; escapes are --token/$HUB_TOKEN and --allow-new-token/$WORKSPACER_ALLOW_NEW_TOKEN=1). Its desktop twin (hubDaemon.ts) only warns — an Electron app that will not boot leaves nobody able to read the message.
- vapid.json → warn + continue + DROP the dead subscriptions. push.New's error is fatal to the WHOLE hub, so refusing would stop the bus/sessions/federation over a notifications keypair. `push-subscriptions.json` is the precise evidence and the damage count: those subs were negotiated against the vanished public key and can never receive again.
- config.yaml → warn + continue, persistence NOT blocked. The brain is a SUPERVISED CHILD; exiting is a restart crash-loop that takes the node's whole capability plane down. Blocking persistence would break the legitimate "config dir has other state but never had a config.yaml" install.

Note: loadFromDisk already handled unreadable / unparseable / empty / MID-RUN disappearance (c.current != nil) with persistBlocked. Only the first read of the process was silent.</observation>
<impact>Any headless deployment where the state dir is a mounted volume: a wrong mount previously presented as a healthy process with a new identity or a factory config.</impact>
<recommendation>New create-once state files should route their ENOENT arm through statelost.Suspected and pick refuse/warn by asking "is this process a foreground CLI, a supervised child, or a GUI?" rather than by how important the file feels.</recommendation>
<related_paths>["services/hub/internal/statelost/*.go", "services/hub/cmd/workspacer/token.go", "services/hub/internal/push/push.go", "services/hub/cmd/brain/config.go", "apps/desktop/src/main/lib/stateLoss.ts"]</related_paths>
<suggested_doc>config</suggested_doc>
<confidence>high</confidence>
