# Workspacer

An Electron desktop application providing a horizontal-scroll workspace for agent-driven development. Built with React and TypeScript.

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

- Node.js 18+
- npm
- Electron (installed via npm)

### Install dependencies

```bash
npm install
cd src/renderer && npm install
```

### Development

```bash
npm run dev
```

This starts both the Vite dev server (frontend) and Electron with hot reload.

### Build

```bash
npm run build
```

### Package for distribution

```bash
npm run package
```

Uses electron-builder to create platform-specific installers. Outputs to `release/` directory:
- **Windows** - NSIS installer, portable executable
- **macOS** - DMG
- **Linux** - AppImage, deb

## Architecture

This is an Electron app with a two-process architecture:

- **Main process** (`src/main/`) - Node.js backend handling window management, IPC, and system services
- **Renderer process** (`src/renderer/`) - React frontend bundled with Vite

## Project Structure

```
src/
  main/           # Electron main process
    index.ts      # App entry point, window creation
    ipc.ts        # IPC handlers
    services/     # Terminal, session, config services
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
npm run test           # Run all tests
npm run test:main      # Main process tests
npm run test:renderer  # Renderer tests
npm run test:e2e       # End-to-end tests (Playwright)
```

## License

MIT
