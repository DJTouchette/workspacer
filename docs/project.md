# WORKSPACER

**A Horizontal-Scroll IDE for Agent-Driven Development**

Project Guide v0.1

---

| | |
|---|---|
| **Framework** | Wails v2 (Go + Web Frontend) |
| **Frontend** | React + TypeScript |
| **Platforms** | Linux (Arch first), macOS, Windows |
| **Author** | Damien |

---

## Vision

Workspacer is a cross-platform desktop application that presents multiple development tools (terminals, code editor, browser, AI agent chat) as cards in an infinitely scrollable horizontal strip. Think of it as a tiling window manager scoped to a single app window, purpose-built for agent-driven coding workflows.

The app targets **Linux (Arch/Hyprland first), macOS, and Windows** as equal citizens. Linux is the primary development and dogfooding platform, with macOS and Windows tested and supported for every release.

---

## Why Wails

Wails v2 wraps Go backend logic and a web frontend into a single native binary. It uses the OS-native webview (WebKitGTK on Linux, WebKit on macOS, WebView2 on Windows) instead of bundling Chromium, resulting in binaries under 10MB versus Electron's 150MB+.

| Advantage | Detail |
|---|---|
| **Go backend** | Fits existing Go knowledge. PTY management, process spawning, file system ops all in Go. |
| **Native webview** | No Chromium shipped. Uses WebKitGTK (Linux), WebKit (macOS), WebView2 (Windows). |
| **Small binary** | Final binary under 10MB. Fast startup, low memory footprint. |
| **React frontend** | Full React + TypeScript for UI. Leverage xterm.js, Monaco, and the existing web ecosystem. |
| **Auto-bindings** | Public Go methods auto-generate TypeScript bindings. Event bridge for real-time communication. |
| **Cross-platform** | Single codebase builds native binaries for all three platforms from one machine. |

---

## Platform Strategy

Linux (Arch) is the primary development platform. All three OS targets are first-class.

| Platform | Webview Engine | Notes |
|---|---|---|
| **Linux (Arch)** | WebKitGTK | Primary dev platform. Hyprland/Wayland native. Needs `webkit2gtk` and `gtk3` packages. AUR package target. |
| **macOS** | WebKit | Native WebKit, zero extra deps. Universal binary (ARM + Intel). App bundle (.app) distribution. |
| **Windows** | WebView2 | Ships with Windows 10/11. MSI or portable .exe distribution. WebView2 runtime auto-installed. |

---

## Architecture

### Go Backend (app.go)

The Go layer handles everything that needs OS-level access. Public methods on the App struct are auto-bound as TypeScript functions on the frontend.

- **PTY management:** spawn and manage pseudo-terminal sessions using `creack/pty`. Each terminal pane gets its own PTY goroutine.
- **Process lifecycle:** start, stop, and monitor child processes (language servers, dev servers, build tools).
- **File system operations:** read, write, watch files. Power the editor's file tree and save functionality.
- **Agent orchestration:** receive commands from the AI agent pane, dispatch actions to terminals or the editor, return results.
- **Event bridge:** Wails runtime events for real-time PTY output streaming to the frontend (Go emits, React listens).

### React Frontend

The frontend is a React + TypeScript SPA rendered in the native webview. It owns all UI layout, theming, and pane management.

- **Horizontal scroll container:** CSS scroll-snap with flex children. Each pane is a fixed-width card. Infinite scroll left/right.
- **Terminal panes:** xterm.js instances connected to Go PTY sessions via the Wails event bridge.
- **Editor pane:** Monaco editor with Go file system bindings for open/save. Language server protocol support via the Go backend.
- **Browser pane:** embedded webview or iframe for documentation/preview. URL bar with navigation.
- **Agent pane:** chat interface that sends prompts to an LLM API, receives tool-call responses, and dispatches actions to other panes.
- **Notes pane:** markdown scratchpad with local persistence.

### Communication Flow

React calls Go methods directly via auto-generated TypeScript bindings (request/response). For streaming data like terminal output, Go emits Wails runtime events that React subscribes to. This gives you both RPC-style calls and real-time push without websockets or HTTP servers.

---

## Key Dependencies

| Layer | Package | Purpose |
|---|---|---|
| Go | `wailsapp/wails/v2` | Application framework, webview bindings, build tooling |
| Go | `creack/pty` | Cross-platform pseudo-terminal allocation for terminal panes |
| Go | `fsnotify/fsnotify` | File system watching for editor live-reload |
| Frontend | `xterm.js` | Terminal emulator in the browser. Connects to Go PTY via events |
| Frontend | `Monaco Editor` | VS Code's editor component. Syntax highlighting, intellisense |
| Frontend | `React 18+` | UI framework with TypeScript |
| Build | `Wails CLI` | Project scaffolding, dev server with hot-reload, cross-compilation |

---

## Project Structure

```
workspacer/
  main.go                 -- Wails app entry point, window config
  app.go                  -- App struct, public methods bound to frontend
  pty.go                  -- PTY session management (creack/pty)
  agent.go                -- Agent orchestration, tool dispatch
  go.mod
  wails.json              -- Wails project config
  build/                  -- Build artifacts, platform installers
  frontend/
    src/
      App.tsx             -- Root component, horizontal scroll container
      panes/
        TerminalPane.tsx  -- xterm.js wrapper
        EditorPane.tsx    -- Monaco wrapper
        BrowserPane.tsx   -- Webview/iframe pane
        AgentPane.tsx     -- AI chat interface
        NotesPane.tsx     -- Markdown scratchpad
      hooks/
        useWailsEvent.ts  -- Subscribe to Go runtime events
        usePTY.ts         -- Terminal session hook
      lib/
        wailsBindings.ts  -- Auto-generated Go method bindings
    package.json
    tsconfig.json
    vite.config.ts
```

---

## Development Phases

### Phase 1: Shell (Week 1-2)

Get the Wails project running with the horizontal scroll layout and placeholder panes.

1. Scaffold Wails project with React TypeScript template
2. Implement horizontal scroll container with CSS scroll-snap
3. Create pane wrapper component with header, resize handles
4. Keyboard navigation (Cmd/Ctrl+1-N, Alt+arrows)
5. Bottom scroll indicator and top nav bar
6. Test on Arch Linux (Hyprland), verify WebKitGTK rendering

### Phase 2: Terminals (Week 3-4)

Wire up real terminal sessions. This is the hardest integration piece.

1. Integrate `creack/pty` on the Go side for PTY allocation
2. Stream PTY output to frontend via Wails runtime events
3. Integrate xterm.js on the React side, connect to PTY stream
4. Handle terminal resize (SIGWINCH propagation)
5. Support multiple concurrent terminal sessions
6. Test shell compatibility (zsh, bash, fish)

### Phase 3: Editor + Browser (Week 5-6)

- Embed Monaco editor with Go file system bindings
- File tree sidebar within the editor pane
- Syntax highlighting, basic intellisense
- Browser pane with URL navigation
- Notes pane with local persistence

### Phase 4: Agent Layer (Week 7-8)

- Agent chat pane with LLM API integration (Anthropic, OpenAI, or local)
- Tool-calling framework: agent can run terminal commands, edit files, search browser
- Action dispatch system: agent responses trigger actions in other panes
- Context awareness: agent sees terminal output, editor contents, browser state

### Phase 5: Cross-Platform + Polish (Week 9-10)

- macOS build testing, universal binary (ARM + Intel)
- Windows build testing, WebView2 runtime handling
- Platform-specific packaging (AUR PKGBUILD, .app bundle, MSI/portable .exe)
- Theming system, keyboard shortcut customization
- Performance profiling, memory optimization

---

## Quick Start

### Arch Linux

```bash
sudo pacman -S go webkit2gtk gtk3 nodejs npm
go install github.com/wailsapp/wails/v2/cmd/wails@latest
wails doctor
wails init -n workspacer -t react-ts
cd workspacer && wails dev
```

### macOS

```bash
brew install go node
go install github.com/wailsapp/wails/v2/cmd/wails@latest
wails doctor
wails init -n workspacer -t react-ts
cd workspacer && wails dev
```

### Windows

```
Install Go from go.dev, Node from nodejs.org
go install github.com/wailsapp/wails/v2/cmd/wails@latest
wails doctor
wails init -n workspacer -t react-ts
cd workspacer && wails dev
```

---

## Build Commands

| Command | Description |
|---|---|
| `wails dev` | Development mode with hot-reload |
| `wails build` | Production build for current OS |
| `wails build -platform linux/amd64` | Linux build (from any OS) |
| `wails build -platform darwin/universal` | macOS universal binary |
| `wails build -platform windows/amd64` | Windows build |

---

## Risks & Considerations

- WebKitGTK version differences across Linux distros may cause rendering inconsistencies. Pin minimum supported version.
- xterm.js + PTY streaming performance under heavy output (e.g. large build logs). May need buffering/throttling on the event bridge.
- Monaco editor bundle size is significant (~5MB). Lazy-load to keep initial startup fast.
- Wails v3 is in development. Current v2 API is stable but monitor for migration path.
- Cross-compilation from Linux to macOS/Windows requires CGo cross-compilers. CI/CD with platform-specific runners is simpler.
- The browser pane in a webview-in-a-webview has limitations. Consider using Go's HTTP handler to proxy content.

---

## Alternatives Considered

| Option | Pros | Why Not |
|---|---|---|
| **Electron** | Massive ecosystem, battle-tested | Ships Chromium (~150MB), high RAM usage, against project goals |
| **Tauri v2** | Rust core, same webview approach | Rust adds learning curve, Go is already in the stack |
| **Flutter** | Native rendering, mobile support | Dart ecosystem, weaker terminal/editor libraries |
| **GPUI (Zed)** | GPU-accelerated, built for IDEs | Early stage, Linux support maturing, Rust-only |``
