---
title: statelost.Suspected counted a bare mkdir as evidence, so the Fly node printed a false STATE LOSS on every first boot
date: 2026-08-25
confidence: high
promoted: false
---

# statelost.Suspected counted a bare mkdir as evidence, so the Fly node printed a false STATE LOSS on every first boot

## Observation
internal/statelost.Suspected returned true for ANY directory entry other than the missing file, including an EMPTY subdirectory. deploy/fly/node/bootstrap.sh mkdir -p's plugins/, library/, layouts/, sessions/ and logs/ inside ~/.config/workspacer before the brain starts, so cmd/brain/config.go's `statelost.Suspected(configDir(), "config.yaml")` fired on every genuinely-first boot of the node: `brain: STATE LOSS: …/config.yaml is missing`. Reproduced against a real image on an empty volume, not inferred. Fixed by requiring an entry to HOLD something (a file of any size, or a non-empty directory). deploy/fly/hub/bootstrap.sh had already reached the same correction independently in shell (its bs_snapshot_dir, "TRAP 1"), so the Go and shell halves of one rule were disagreeing.</observation>
<parameter name="impact">A guard that is wrong on every first boot is one the operator learns to scroll past, which costs it the cases it exists for. Also: apps/desktop/src/main/lib/stateLoss.ts is the TS twin and MUST be changed in step — both halves mint the same remote-token. A second trap: that TS file must use `fs.readdirSync(dir)` (name list), NOT `{ withFileTypes: true }` — hubDaemon's test doubles mock readdirSync as a plain string array, so depending on Dirent breaks 20 unrelated tests.</impact>
<parameter name="recommendation">When changing statelost, change both twins and both test tables together. Empty directories are never evidence; zero-byte FILES are.</recommendation>
<parameter name="related_paths">["services/hub/internal/statelost/*.go", "apps/desktop/src/main/lib/stateLoss*.ts", "deploy/fly/*/bootstrap.sh", "services/hub/cmd/brain/config.go"]
