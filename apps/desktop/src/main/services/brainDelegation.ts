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
