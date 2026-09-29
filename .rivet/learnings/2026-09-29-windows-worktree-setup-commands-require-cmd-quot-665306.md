---
title: Windows worktree setup commands require cmd quoting rather than CRT argv quoting
date: 2026-09-29
promoted: false
---

# Windows worktree setup commands require cmd quoting rather than CRT argv quoting

## Observation
The Windows full backend CI at599be31f passed timeout commands without quotes but failed the first quoted SOURCE path setup command. run_setup passed shell source through Command.args, applying CRT quote escaping that cmd.exe does not decode. Use CommandExt::raw_arg for the complete owner-configured shell source with /D /S /C and outer quotes; SOURCE and WORKTREE remain environment data. Rust primary documentation: https://doc.rust-lang.org/std/os/windows/process/trait.CommandExt.html#tymethod.raw_arg. Actual Windows CI rerun remains required; Linux tests cannot validate this branch.
