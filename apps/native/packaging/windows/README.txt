Workspacer Native (experimental) - Windows x64

Launch Workspacer Native from the Start menu to start the native GUI and its
local backend. The installer includes the service binaries, shared service
bundle, private Node runtime and Microsoft C++ runtime DLLs. It does not need
Electron or a separately installed Node runtime for shared services.

Install and sign in to Claude Code or Codex separately to run agents. Install
Git for project operations. Close Workspacer Native before upgrading. Local
mode needs ports 7895 and 7897; close another Workspacer backend using these
ports first, or connect to that backend instead.

From this directory:
  .\wks-native.exe --local
  .\wks-native.exe --bus ws://127.0.0.1:7895/bus

Opening wks-native.exe directly without --local connects to an existing hub.
The Start menu shortcut supplies --local for a self-contained launch.

This native installer is unsigned. Updates are manual: download the next
Workspacer-Native-Setup-<version>-x64.exe and run it after closing the app.
The Electron app's updater does not update this installation.

Uninstall through Windows Settings > Apps. Session data and shared Workspacer
configuration are retained. The native app's database is separate from the
Electron app's database; shared configuration is still shared.
