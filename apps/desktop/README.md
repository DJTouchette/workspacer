# Workspacer — Desktop app

The Electron + React GUI client. Part of the [Workspacer monorepo](../../README.md);
sibling daemons (`claudemon`, `hub`) live in `../../services/` and are spawned by
this app at runtime unless it adopts or connects to an already-running
`workspacer serve`.

For the detailed feature/maturity matrix, see
[docs/features.md](../../docs/features.md). This README covers the desktop
client surface and local development.

## Features

- **Horizontal pane layout** - Work across terminals, agents, browser/webview panes, notes, editor, review, settings, inspector, and agent-watch panes
- **Agent sidebar and Inbox** - Keep the current agent visible while approval/question items collect in the Triage Inbox
- **Claude and managed-agent integration** - Claude PTY/stream sessions plus managed Codex, OpenCode, and Pi sessions through claudemon
- **Review workflow** - Inspect git status/diffs, stage or unstage files, commit, push, and return to the agent
- **Session persistence** - Auto-save and resume workspaces across restarts
- **Browser hibernation** - Inactive browser panes hibernate to save resources
- **Nerd Font support** - Auto-discovers and injects local Nerd Fonts
- **Configurable keybindings** - Default and Vim-style modes with leader key support
- **Command palette** - Quick access to apps, panes, Ask the Fleet, phone access, and server connection actions
- **Phone and web access** - Share this desktop with `/m` phone access, `/remote` terminal mirror, or `/app/`; connect the desktop shell to another Workspacer server when needed
- **Advanced overview** - Agent Overview and Agent Monitor remain available for larger multi-agent runs without being required for the main workflow

## First task

The packaged desktop app starts its own runtime. You do not need Node, Go, Rust,
or the Workspacer CLI to use it.

1. Install a supported provider CLI on the machine where the agent will run
   (for example Claude Code or Codex), then complete that provider's account
   sign-in in its CLI. Workspacer uses your provider account; finding a binary
   does not verify authentication. The Guide specifically requires Claude Code.
2. Choose **Start your first task**, describe one bounded task, and choose an
   existing accessible folder. Git folders can optionally use a separate
   worktree in Advanced. Non-git folders work without branch isolation.
   Use an absolute path: a typed `~` is not expanded. On a remote target, the
   folder and provider must exist on that machine.
3. Keep the default permission mode, which requests approval where the provider
   requires it. Full access is an explicit opt-in. Send the task once; if launch
   fails, the task and folder remain available for retry. **Check runtime again**
   only reads status; it does not restart anything or sign in for you.
4. Follow the agent's chat for replies and results. Approval requests and
   questions appear in chat and Inbox. Fleet shows your agents and available
   summaries; **Back to fleet** preserves the open chat and draft. A prose reply
   is still a reply, and a malformed result is not proof of success.
5. Use **Resume** on a stopped session to continue its existing conversation.
   Conversation history can return after restarting the desktop; this does not
   promise persistence of the Fleet Manager's live coordination graph.

Use a **direct agent** for a single bounded task. Ask the **Fleet Manager** in
Overview to coordinate work across projects; it also needs the hub and its
Workspacer action tools. When only those services are degraded, a supported
direct local launch may still work. A runtime-ready label is not a guarantee of
provider authentication or task success. Startup failures appear in system
notices with an **Open logs** action.

Open **Help: First task** in the command palette to return to this page.
**Settings → Command Line** installs the optional Workspacer CLI for terminal use
and `workspacer serve`; it does not install or authenticate provider CLIs.

## Local development

### Prerequisites

- Node.js 22 (repo pins it via `mise`)
- npm
- Go 1.25 and Rust (to build the bundled hub and claudemon daemons)

### Install dependencies

From the repo root:

```bash
make install            # == cd apps/desktop && npm install
cd apps/desktop/src/renderer && npm install
```

### Development

From the repo root:

```bash
make dev                # or ./dev   (== cd apps/desktop && npm run dev)
```

This builds the `hub` binary, starts the Vite dev server, and launches Electron
with hot reload.

### Remote / phone access

The normal sharing flow is phone-first: open **Phone access** from the command
palette to show the QR/link for `/m`. The same dialog exposes the lightweight
terminal mirror at `/remote`, the full web renderer at `/app/`, scoped pairing
tokens, and the advanced "connect this desktop to another server" client mode.

When `workspacer serve` is already healthy on the same machine, the desktop
adopts it instead of starting another claudemon/hub pair. When "Connect to
Server..." is configured, the renderer uses that remote server's bus while
host-shell actions stay local to the Electron process.

### Build / Package

```bash
make build              # build this app (+ daemons via the root target)
make package            # build daemons + electron-builder installers -> apps/desktop/release/
```

## Architecture

Two-process Electron app:

- **Main process** (`src/main/`) - Node.js backend: window management, IPC, system
  services, and supervision of the `claudemon` / `hub` daemons in `../../services/`
- **Renderer process** (`src/renderer/`) - React frontend bundled with Vite

```
src/
  main/           # Electron main process
    index.ts      # App entry point, window creation
    ipc.ts        # IPC handlers
    services/     # Terminal, session, config, claudemon/hub daemon supervision
  renderer/       # React frontend (Vite)
    src/
      App.tsx     # Main app component
      components/ # UI components (NavBar, ScrollContainer, etc.)
      panes/      # Pane implementations (Terminal, Browser, Claude, etc.)
      hooks/      # React hooks (useTabManager, useKeyboardNav, etc.)
```

## Keyboard Shortcuts

Press `?` to view the shortcut overlay. Default bindings include:

- `Ctrl+T` - New terminal tab
- `Ctrl+W` - Close current tab
- `Ctrl+[1-9]` - Jump to tab by number
- `Ctrl+Shift+Left/Right` - Move tab

## Testing

```bash
npm run test           # Run all tests (from apps/desktop)
npm run test:main      # Main process tests
npm run test:renderer  # Renderer tests
npm run test:e2e       # End-to-end tests (Playwright)
```
