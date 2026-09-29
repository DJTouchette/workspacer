/** The shared Rust control plane owns the hub and MCP transports. Electron
 * provides its catalog and live UI services as one external capability provider. */
export const RUST_HUB_ENABLED = true;
export const DELEGATE_CATALOG_TO_BRAIN = false;

/**
 * Whether the *desktop renderer* should consume the hub bus (the brain + main
 * as the providers) instead of talking to main directly over IPC — mirroring
 * the TUI, which defaults to the bus with `--direct` to opt out. One source of
 * truth: in bus mode the renderer drives and observes agents through the same
 * bus the web/remote client uses, so desktop and web run the identical
 * transport against the identical providers.
 *
 * Only the data/orchestration/observation plane moves to the bus; genuinely
 * host-only desktop concerns (native dialogs, plugin management, OS
 * notifications, window chrome, MessagePort terminal-exit) stay on IPC — see
 * the renderer's bridgedBackend, which keeps those on the real preload.
 *
 * Default ON. Toggle back to the prior pure-IPC behavior with
 * WORKSPACER_DESKTOP_DIRECT=1 (the desktop analogue of the TUI's `--direct`).
 */
export const DESKTOP_RENDERER_USES_BUS = process.env.WORKSPACER_DESKTOP_DIRECT !== '1';
