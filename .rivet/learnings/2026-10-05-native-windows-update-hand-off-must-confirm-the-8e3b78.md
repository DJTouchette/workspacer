---
title: Native Windows update hand-off must confirm the helper before quitting
date: 2026-10-05
confidence: high
suggested_doc: auto-update-release-channel
related_paths:
  - apps/native/src/updates.rs
  - apps/native/src/update_helper.ps1
  - apps/native/src/ui/updater.rs
  - apps/native/tests/windows_update_handoff.rs
  - apps/native/packaging/windows/installer.nsi
promoted: false
---

# Native Windows update hand-off must confirm the helper before quitting

## Observation
apps/native self-update quits only after the PowerShell helper (src/update_helper.ps1) records state 'waiting' with the hand-off nonce in %LOCALAPPDATA%\Workspacer Native Rust Preview\updates\last-update.json. The script is constant text: every path and argument comes from a JSON plan named by WKS_UPDATE_PLAN, so the script carries no double quotes or user data (unit-tested). Spawn uses CREATE_NO_WINDOW|CREATE_NEW_PROCESS_GROUP plus CREATE_BREAKAWAY_FROM_JOB, retrying without breakaway on ERROR_ACCESS_DENIED. DETACHED_PROCESS must not be combined with CREATE_NO_WINDOW, because Windows then ignores CREATE_NO_WINDOW. The install runs as /S /D=<exe folder> and requires exit 0 plus the expected build-stamp.json version. NSIS silent mode defaults a file-write error to Ignore (script.cpp: IDIGNORE<<21), so a locked file yields a partial install with exit 1 via installer.nsi's ${Errors} check. The helper therefore refuses to install while any process runs from the install folder.

## Impact
The pre-fix hand-off quit right after spawn() with silenced errors, unconditional relaunch and an unrendered notice. A dead helper (no console, killed with the app's job) closed the app and left no trace (#23). A build that already contains the old hand-off cannot be fixed remotely; its users must reinstall once.

## Recommendation
Keep values out of the script text and keep the readiness handshake. Test Windows behavior with tests/windows_update_handoff.rs (harness=false; copies of the test exe act as the app, installer and busy process). Its evidence comes only from the windows-latest cargo test steps. Run UI hand-off tests with the injectable Extras.update_starter, never the real helper.
