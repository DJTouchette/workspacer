# Phase 2: Terminals

Wire up real terminal sessions. This is the hardest integration piece.

---

## Go PTY Backend

- [ ] Research PTY approach for cross-platform support
  - `creack/pty` for Linux/macOS
  - ConPTY for Windows (may need `UserExperienceVirtualization/conpty` or direct syscalls)
  - Consider a unified abstraction layer over both
- [ ] Implement PTY session manager in Go
  - Spawn new PTY sessions with configurable shell (zsh, bash, fish, powershell, cmd)
  - Track active sessions by ID
  - Clean up sessions on close
- [ ] Expose public Go methods for frontend bindings
  - `CreateTerminal(shell string) string` — returns session ID
  - `WriteTerminal(id string, data string)` — send input to PTY
  - `ResizeTerminal(id string, cols int, rows int)` — handle resize
  - `CloseTerminal(id string)` — kill session

## PTY Output Streaming

- [ ] Stream PTY stdout to frontend via Wails runtime events
- [ ] Event format: `terminal:{id}:output` with data payload
- [ ] Buffering/throttling for high-throughput output (build logs, large file cats)
- [ ] Handle PTY process exit — notify frontend, clean up

## Frontend Terminal Panes

- [ ] Integrate xterm.js into TerminalPane component
- [ ] Connect xterm.js input to Go `WriteTerminal` binding
- [ ] Subscribe to PTY output events via `useWailsEvent` hook
- [ ] Create `usePTY` hook encapsulating session lifecycle
- [ ] Terminal resize: detect xterm.js fit dimensions, call `ResizeTerminal`
- [ ] SIGWINCH propagation on Linux (window resize signals)
- [ ] Multiple concurrent terminal sessions (each pane = own session)
- [ ] New terminal button/shortcut to spawn additional terminal panes

## Shell & Editor Compatibility

- [ ] Test with bash, zsh, fish on Linux
- [ ] Test with PowerShell, cmd on Windows
- [ ] Test neovim running inside the terminal pane — verify:
  - Cursor movement and modes
  - Syntax highlighting / true color
  - Mouse support
  - Split panes within neovim
  - Plugin compatibility (telescope, etc.)
- [ ] Test other TUI apps (htop, lazygit, etc.)

## Performance

- [ ] Profile terminal rendering under heavy output (e.g., `find /`, large compile)
- [ ] Verify no input latency — keystrokes should feel instant
- [ ] Memory usage with 5+ concurrent terminal sessions
- [ ] GPU-accelerated rendering in xterm.js (WebGL addon)

## Cross-Platform

- [ ] Verify PTY works on Arch Linux (Wayland)
- [ ] Verify ConPTY works on Windows 10/11
- [ ] Consistent terminal behavior across both platforms
- [ ] Default shell detection per platform
