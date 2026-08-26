---
title: The Fly node's operator token reaches /bin/sh inside the hub process via jobs.upsert + jobs.run
date: 2026-08-25
confidence: high
promoted: false
---

# The Fly node's operator token reaches /bin/sh inside the hub process via jobs.upsert + jobs.run

## Observation
The node attaches with an operator-tier HUB_TOKEN; operator tier is `trusted`; jobsTrusted (services/hub/cmd/hub/main.go:329) is a bare c.IsTrusted() with nothing narrower. So a node may call jobs.upsert then jobs.run, and a job of kind shell reaches jobs.BusRunner.Shell (internal/jobs/scheduler.go:371), which is exec.CommandContext("/bin/sh", "-c", command) — unconfined, in the HUB process's environment, i.e. the one holding $FLY_API_TOKEN, on the volume holding nodes.json/tokens.json/remote-token. An empty --jobs-file is the off switch: the flag defaults to defaultJobsFile() (main.go:383, "Empty = jobs disabled") and `if *jobsFile != ""` (main.go:574) wraps every jobs.* RegisterLocalIdent. Verified executably both ways inside the image: with the empty flag `workspacer jobs list` answers "no provider for jobs.list" (rc 1); the same binary with a non-empty --jobs-file on another port answers normally (rc 0).</observation>
<parameter name="impact">The node is by design the machine that runs agent-written code, so this is a real path from a prompt-injected agent to the credential that creates and destroys machines. deploy/fly/hub/entrypoint.sh now passes --jobs-file "". Do NOT delete that flag to enable jobs: fix the tier or the gate instead.</impact>
<parameter name="related_paths">["services/hub/cmd/hub/main.go", "services/hub/internal/jobs/scheduler.go", "deploy/fly/hub/entrypoint.sh"]
