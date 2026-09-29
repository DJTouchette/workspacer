---
title: Rust worktree cleanup and launch share native admission leases
date: 2026-09-28
promoted: false
---

# Rust worktree cleanup and launch share native admission leases

## Observation
The Rust worktree service uses claudemon WorktreeAdmission reference-counted fences and exclusive WorktreeMaintenance leases around removal. Cleanup rechecks authoritative daemon rows under the lease, including nested live cwd, and never passes --force or deletes the branch. Creation records matching allocation identity before trusted setup and returns a reservation held through Lifecycle::launch_reserved. Setup paths are environment data rather than textual shell interpolation so punctuation in a caller-selected directory cannot become shell code. Scheduled artifact cleanup/reference scans and workflow review-evidence capture remain separate unported behavior.
