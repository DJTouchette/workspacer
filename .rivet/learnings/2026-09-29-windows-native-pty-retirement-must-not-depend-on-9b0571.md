---
title: Windows native PTY retirement must not depend only on ConPTY EOF
date: 2026-09-29
promoted: false
---

# Windows native PTY retirement must not depend only on ConPTY EOF

## Observation
Checkpoint 04322791 actual Windows tests proved std, Tokio and ConPTY immediate-fork job cleanup, but the real worktree test timed out waiting for a signaled native PTY to leave live state. daemon/spawn.rs previously retired its generation only after output-channel EOF, while the retained portable-pty master owns HPCON. Windows native output pumping now observes the actual owned child exit and permits a fixed 250 ms final output drain before existing handle-identity and generation-fenced teardown. Signal submission alone never authorizes retirement; Unix EOF behavior is unchanged. A retained-master and successor-generation Windows regression is explicitly invoked by rust-native-preview.yml; a separate exact-source probe covers bounded-drain tests. Actual Windows execution of this follow-up remains required.
