---
title: Stop the hub command receiver before joining observers that may await startup replies
date: 2026-09-29
confidence: high
suggested_doc: native-embedded-backend
related_paths:
  - services/hub-rs/src/runtime.rs
  - services/hub-rs/src/services/library_watch.rs
  - services/hub-rs/tests/visible_terminal.rs
promoted: false
---

# Stop the hub command receiver before joining observers that may await startup replies

## Observation
A real visible-terminal shutdown fixture sometimes hung after Hub.shutdown even in an isolated child process. Stage watchdog located hub shutdown. The actor had exited its pump loop but kept the receiver alive while joining the library watcher, whose uncancelled Client::connect could be queued awaiting a reply from the departed actor. Closing, draining and dropping the receiver immediately after loop exit cancels queued oneshot replies before observer joins; plugin stop still occurs while the actor pumps. A production Handle connection queue regression and repeated bounded real lifecycle children validate the fix; Full library382 passed, including the queued-connection regression; dedicated visible-terminal integration passed8 fresh child lifecycles, and lifecycle8/plugin-manager21 passed. Logs are /tmp/workspacer-shutdown-final-lib.log, /tmp/workspacer-shutdown-visible-final.log and /tmp/workspacer-shutdown-ordering-final.log.
