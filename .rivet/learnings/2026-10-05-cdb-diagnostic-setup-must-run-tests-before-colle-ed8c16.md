---
title: CDB diagnostic setup must run tests before collecting AV stacks
date: 2026-10-05
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/native/scripts/windows-gpui-crash-diagnostics.ps1
promoted: false
---

# CDB diagnostic setup must run tests before collecting AV stacks

## Observation
CDB -c runs at debugger startup, so a bare stack/quit command exits before the suite. Install sxe -c AV commands at the initial breakpoint then g; source Microsoft cdb-command-line-options and debuggercmds sx set-exceptions documentation. Portable pwsh parsing validates syntax only, not Windows debugger behavior.
