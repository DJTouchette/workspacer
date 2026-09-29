---
title: Confirmed controls and terminal contract state are distinct host facts
date: 2026-09-29
promoted: false
---

# Confirmed controls and terminal contract state are distinct host facts

## Observation
Go livecontrol noteLiveControl creates metadata for hook/manual sessions too; Rust initially preserved controls only for coordinator launch journal rows. The local confirmed-control cache now stamps real observed identity before requests, accepts only same-stamp ACK bookkeeping and resets on SessionStart/end/disappearance/identity change, without creating launch or credential authority. Finish wake audit also found delivery.await? aborting the completion group after escalation failure, unlike Go independent sends; errors are now accumulated while other groups proceed. TS supervisorNudge validated() separates valid schema contracts/escalation from FAILED prose: Rust no longer invalidates valid contracts merely because the reply also has the provider error marker, and retains normalized escalation JSON outcome. Focused regression tests added; platform checkpoint runs them next.
