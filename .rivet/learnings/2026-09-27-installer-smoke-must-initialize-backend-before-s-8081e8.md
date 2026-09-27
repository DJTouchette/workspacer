---
title: Installer smoke must initialize backend before settings services
date: 2026-09-27
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/native/scripts/test-windows-installer.ps1
  - apps/native/packaging/windows/installer.nsi
promoted: false
---

# Installer smoke must initialize backend before settings services

## Observation
Windows release run 36349842906 built the native installer, but its standalone pricing service probe created settings before the embedded backend minted remote-token. The launcher correctly refused that partial state as credential loss. Run the embedded probe first, then desktop-host protocol checks. NSIS also accepted malformed dollar-quote uninstall registry strings as warnings; use ordinary embedded double quotes inside single-quoted NSIS strings, compile with /WX, and assert both uninstall registry command strings in the Windows smoke.
