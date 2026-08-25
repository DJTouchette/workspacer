---
title: TestAskingDoesNotMakeTheMachineBusy (cmd/hub quiescence_test.go) is flaky at HEAD
date: 2026-08-24
confidence: high
promoted: false
---

# TestAskingDoesNotMakeTheMachineBusy (cmd/hub quiescence_test.go) is flaky at HEAD

## Observation
`go test ./cmd/hub/ -run TestAskingDoesNotMakeTheMachineBusy` fails most runs on master as of 1840319d, with "a poller that went on to do real work must count again: []". Verified in a clean `git worktree` at HEAD with no local changes: 5 failures out of 5. In the working tree it passed 1 run in 3, so it is flaky rather than uniformly broken.</observation>
<parameter name="impact">It is the only red test in `services/hub`, so anyone running the full Go suite sees a failure that is not theirs, and CI's hub leg should be red on master.</impact>
<parameter name="recommendation">Belongs to the fleet.quiescence work (84daf059). Fix it there rather than in unrelated branches; check whether the test depends on wall-clock timing around the poller-reclassification window.</recommendation>
<parameter name="related_paths">["services/hub/cmd/hub/quiescence_test.go", "services/hub/internal/quiescence/*"]
