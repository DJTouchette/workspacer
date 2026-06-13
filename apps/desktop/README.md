# Workspacer — Desktop app

The Electron + React GUI client. Part of the [Workspacer monorepo](../../README.md);
sibling daemons (`claudemon`, `hub`) live in `../../services/` and are spawned by
this app at runtime.

## Features

- **Horizontal pane layout** - Navigate between panes with keyboard shortcuts or scroll
- **Multiple pane types** - Terminal, browser, notes, Claude Code, and settings
- **Claude Code integration** - Built-in Claude CLI panes with headless terminal mirroring and hook support
- **Session persistence** - Auto-save and resume workspaces across restarts
- **Browser hibernation** - Inactive browser panes hibernate to save resources
- **Nerd Font support** - Auto-discovers and injects local Nerd Fonts
- **Configurable keybindings** - Default and Vim-style modes with leader key support
- **Command palette** - Quick access to apps and actions

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
