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

## Getting Started

### Prerequisites

- Node.js 18+ (repo pins Node 22 via `mise`)
- npm
- Go 1.25 (to build the `hub` binary the app launches)

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
