---
title: Native GPUI client has Windows CI but no release installer
date: 2026-09-27
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - .github/workflows/native-client.yml
  - .github/workflows/release.yml
  - apps/desktop/electron-builder.yml
promoted: false
---

# Native GPUI client has Windows CI but no release installer

## Observation
The native-client workflow builds wks-native on Windows, macOS and Linux but uploads only a Linux smoke screenshot; it does not package or publish native executables. release.yml packages apps/desktop with electron-builder, whose Windows NSIS and portable targets are the Electron app. The GPUI apps/native client is not wired into the release workflow.
