# Phase 1: Shell

Get the Wails project running with the horizontal scroll layout and placeholder panes on both Linux and Windows.

---

## Setup

- [x] Install Wails v3 CLI and verify (Linux)
- [x] Scaffold project with `wails3 init -n workspacer -t react-ts`
- [x] Verify the scaffold builds on Arch Linux (8.8MB binary)
- [x] Set up project structure (frontend/src/components/, panes/, hooks/, types/)
- [x] Go backend: WorkspaceService with PaneConfig model
- [x] YAML config system (~/.config/workspacer/config.yaml)
- [x] ConfigService exposing config to frontend via bindings
- [ ] Test on Windows — verify WebView2 rendering + build

## Horizontal Scroll Layout

- [x] Implement horizontal scroll container (ScrollContainer.tsx)
- [x] CSS scroll-snap for card-to-card snapping (scroll-snap-type: x mandatory)
- [x] Dynamic pane width based on viewport and `peek` config
- [x] Bottom scroll position indicator (ScrollIndicator.tsx — dots with active highlight)
- [x] Top nav bar with pane labels/icons for quick jumping (NavBar.tsx)
- [x] Auto-detect active pane based on scroll position (closest to center)
- [x] "+" button in NavBar to add new terminal panes

## Pane Component

- [x] Generic Pane wrapper component with header + content area
- [x] Pane header with type indicator, title, and close button
- [x] Active pane border highlight (blue accent + box shadow)
- [x] Placeholder content for each pane type (terminal, browser, notes, agent)
- [x] Pane resize handles (drag right edge to adjust width, double-click to reset)
- [ ] Pane reordering via drag-and-drop (stretch goal — deferred)

## Keyboard Navigation

- [x] Ctrl+1-9 to jump to pane by index
- [x] Alt+Left/Right to scroll between adjacent panes
- [x] Pane focus management (visual indicator + terminal focus/blur)
- [x] Ctrl+T to add new terminal pane
- [x] Ctrl+W to close active pane
- [x] Ctrl+/ to toggle keyboard shortcut help overlay
- [x] ShortcutOverlay component with all shortcuts listed

## Config (YAML)

- [x] ui.animations, ui.theme, ui.font_family, ui.font_size
- [x] ui.navbar_height, ui.pane_header_height, ui.border_radius
- [x] terminal.shell, terminal.font_family, terminal.font_size
- [x] terminal.scrollback, terminal.cursor_blink, terminal.cursor_style
- [x] panes.default_width, panes.gap, panes.peek
- [x] panes.default (initial pane layout)

## Cross-Platform

- [ ] Test on Windows — verify WebView2 rendering + build
- [ ] Confirm keyboard shortcuts work on Windows
