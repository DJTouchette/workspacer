# Phase 2: Terminals

Wire up real terminal sessions. This is the hardest integration piece.

---

## Go PTY Backend

- [x] Cross-platform PTY via `aymanbagabas/go-pty` (creack/pty on Unix, ConPTY on Windows)
- [x] PTY session manager with mutex-protected sessions map
- [x] Spawn new PTY sessions with configurable shell
- [x] Default shell detection per platform ($SHELL on Unix, pwsh/powershell on Windows)
- [x] Public methods: CreateTerminal, WriteTerminal, ResizeTerminal, CloseTerminal
- [x] ServiceStartup / ServiceShutdown lifecycle

## PTY Output Streaming

- [x] Stream PTY stdout to frontend via Wails runtime events
- [x] Event format: `terminal:{id}:output` with base64 data payload
- [x] Output batching with requestAnimationFrame on frontend
- [x] Handle PTY process exit — emit `terminal:{id}:exit`, clean up session

## Frontend Terminal Panes

- [x] xterm.js TerminalPane with FitAddon
- [x] Connect xterm.js input to Go WriteTerminal (base64 encoded)
- [x] usePTY hook encapsulating session lifecycle
- [x] Terminal resize via ResizeObserver + FitAddon + ResizeTerminal
- [x] Multiple concurrent terminal sessions (each pane = own session)
- [x] Ctrl+T to spawn new terminal panes

## Shell & Editor Compatibility

- [x] Neovim confirmed working (cursor, syntax highlighting, true color)
- [ ] Test with bash, zsh, fish on Linux
- [ ] Test with PowerShell, cmd on Windows
- [ ] Test other TUI apps (htop, lazygit, etc.)

## Performance

- [ ] Profile terminal rendering under heavy output (e.g., `find /`, large compile)
- [ ] Verify no input latency — keystrokes should feel instant
- [ ] Memory usage with 5+ concurrent terminal sessions
- [ ] Re-evaluate WebGL addon (disabled due to WebKitGTK issues)

## Cross-Platform

- [x] PTY works on Arch Linux (Wayland) via creack/pty
- [x] ConPTY support for Windows via go-pty (needs testing)
- [ ] Test on Windows 10/11 — verify terminals work with PowerShell
- [ ] Consistent terminal behavior across both platforms
