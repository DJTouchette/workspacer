Workspacer (macOS)

Drag Workspacer to Applications.

This build is not notarized by Apple, so macOS blocks the first launch of a
downloaded copy. Clear the quarantine flag once, in Terminal:

    xattr -cr /Applications/Workspacer.app

Then open Workspacer normally. It runs its own local engine and picks up the
PATH from your login shell, so Claude Code, Codex, Node and Git are found the
same way as in a terminal. Agent providers are separate installs.

Updates: Workspacer checks for new releases and tells you when one exists;
install it by downloading the new .dmg and replacing the app. Your sessions,
settings and logs are kept.
