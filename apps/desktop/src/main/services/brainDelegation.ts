/**
 * Single source of truth for the brain-delegation decision.
 *
 * When ON, the desktop spawns the hub with `--brain-scope catalog` (see
 * hubDaemon), and main STOPS registering the file-backed "catalog" capabilities
 * (config, profiles, library, layouts, saved sessions, models, session
 * discovery, host file reads — see hubCapabilities). The headless brain provider
 * then owns them as the single provider on the bus — the router is single-owner
 * per method, so two providers would collide.
 *
 * The live/enriched agent + streaming capabilities are unaffected: main still
 * owns those until they move to the brain (the streaming phase).
 *
 * Kill switch: set WORKSPACER_NO_BRAIN=1 to keep main as the provider (e.g. if
 * the packaged brain binary is missing). Then the hub is spawned with
 * `--brain-scope off` and main registers the catalog itself, exactly as before.
 */
export const DELEGATE_CATALOG_TO_BRAIN = process.env.WORKSPACER_NO_BRAIN !== '1';

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
