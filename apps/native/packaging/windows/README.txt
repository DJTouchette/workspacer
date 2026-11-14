Workspacer Native (Windows x64)

Launch Workspacer Native from the Start menu. The shortcut runs
wks-native.exe --local: the app owns its backend (hub, session engine, MCP
tools and desktop services) in its own process. Nothing else has to be
running, and no Node runtime or Electron app is needed.

Install and sign in to Claude Code or Codex separately to run agents, and
install Git for project operations. Agent providers are optional external
integrations, configured separately.

From this folder:
  .\wks-native.exe --local
  .\wks-native.exe --bus ws://127.0.0.1:7895/bus
  .\workspacer-rust.exe --help

wks-native.exe without --local connects to a hub that is already running.
workspacer-rust.exe is the standalone server and command-line client.

Updates
  The app checks for new releases and installs them in place from
  Settings > About. You can also download a newer
  Workspacer-Setup-<version>-x64.exe and run it; it upgrades this
  installation where it is.

Upgrading from "Workspacer Native Rust Preview"
  This is the same app under its final name. Running this installer over a
  preview installation upgrades it in its existing folder and replaces the
  preview entry in Windows Settings > Apps with "Workspacer Native". Your
  sessions and settings are kept.

Uninstall
  Use Windows Settings > Apps. Uninstall preserves sessions and shared
  settings: it removes only the files this installer placed, and leaves your
  data folder under %LOCALAPPDATA% and any files you added to this folder.
