---
title: portable-pty kill is not an owned Unix process-group kill
date: 2026-09-28
suggested_doc: claudemon-pty-wrapper
related_paths:
  - services/claudemon/src/wrapper/pty_owned.rs
  - services/claudemon/src/wrapper/pty_windows_job.rs
promoted: false
---

# portable-pty kill is not an owned Unix process-group kill

## Observation
portable-pty0.8 calls setsid before Unix exec, but ChildKiller::kill initially sends SIGHUP to the direct PID, then force-kills only that PID. The old claudemon SIGKILL wrapper therefore did not establish descendant cleanup. New owned Child wrapper verifies separate session/group and uses waitid(WNOWAIT) to keep the original direct child as a PID anchor until group cleanup precedes reaping; TERM/INT/KILL never target the embedding process group. A verified foreground job group on the same controlling PTY is also signalled. Other background process groups and daemonized setsid descendants remain a Unix limitation. Windows assigns only the spawned child to a kill-on-close job; portable-pty has no suspended-spawn hook, so descendants launched before job assignment are not retroactively guaranteed owned.
