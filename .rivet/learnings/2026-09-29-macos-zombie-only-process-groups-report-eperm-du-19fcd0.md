---
title: macOS zombie-only process groups report EPERM during owned cleanup
date: 2026-09-29
promoted: false
---

# macOS zombie-only process groups report EPERM during owned cleanup

## Observation
Apple XNU bsd/kern/kern_sig.c killpg1 excludes SZOMB members and returns EPERM when no eligible member remains. Therefore Linux-successful waitid(WNOWAIT) cleanup can fail on macOS for an ordinary already-exited command: the deliberately unreaped child is still a zombie group member. child_group::signal retains callers unreaped-anchor fence and handles macOS EPERM only after bounded proc_listpids(PROC_PGRP_ONLY) plus proc_pidinfo(PROC_PIDT_SHORTBSDINFO,arg=1) positively proves every member SZOMB; permission failures and live/unknown members remain errors. Source: https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c and proc_info.c. Mac preview first exposed stdin/HTMLcard/provider-title/Git-ignore failures; runtime validation of new fix awaits platform CI.
