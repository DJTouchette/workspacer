Workspacer Native (Linux)

The native Workspacer client: one binary that owns its Rust hub, engine and MCP
services in process when started with --local, or connects to a hub that is
already running when started without it.

Install for your user (no root needed):

    ./install.sh

That copies the application under ~/.local (set PREFIX to choose another
place, for example PREFIX=/usr/local with sudo), adds a launcher that starts
it with --local, and writes the list of what it installed so ./uninstall.sh
(also installed as wks-native-uninstall) removes exactly those files. Your
sessions, settings and logs are kept.

Run it without installing:

    ./lib/workspacer-native/wks-native --local

Requirements: a Vulkan-capable graphics driver, and X11 or Wayland with
libxkbcommon. Agent providers (Claude Code, Codex) are separate installs.

Updates: this build checks for new releases and tells you when one exists;
install it by unpacking the new archive and running ./install.sh again.
