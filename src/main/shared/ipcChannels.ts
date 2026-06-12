/**
 * Single source of truth for all IPC channel name strings.
 * Both ipc.ts (main) and preload.ts (renderer bridge) import from here so
 * a rename is a single-file change and tsc catches mis-spelled keys.
 *
 * Channel values are identical to the bare strings that existed before — the
 * runtime wire format is unchanged.
 */
export const IPC = {
  // ── Library ──
  LIBRARY_LIST: 'library:list',
  LIBRARY_SAVE: 'library:save',
  LIBRARY_REMOVE: 'library:remove',
  LIBRARY_CHANGED: 'library:changed', // push (main → renderer)

  // ── Notifications / ambient awareness ──
  NOTIFY_SET_ACTIVE_SESSION: 'notify:set-active-session', // send (renderer → main)
  NOTIFY_FOCUS_AGENT: 'notify:focus-agent', // push (main → renderer)

  // ── Generic terminal ──
  TERMINAL_CREATE: 'terminal:create',
  TERMINAL_RESIZE: 'terminal:resize',
  TERMINAL_CLOSE: 'terminal:close',
  TERMINAL_PORT: 'terminal:port',   // push (main → renderer, MessagePort)
  TERMINAL_EXIT: 'terminal:exit',   // push (main → renderer)

  // ── Claude sessions ──
  CLAUDE_SPAWN: 'claude:spawn',
  CLAUDE_LIST_MODELS: 'claude:listModels',
  CLAUDE_MESSAGE: 'claude:message',
  CLAUDE_APPROVE: 'claude:approve',
  CLAUDE_ANSWER: 'claude:answer',
  CLAUDE_RESIZE: 'claude:resize',
  CLAUDE_SIGNAL: 'claude:signal',
  CLAUDE_CLOSE: 'claude:close',
  CLAUDE_ATTACH: 'claude:attach',
  CLAUDE_DETACH: 'claude:detach',
  CLAUDE_GATE: 'claude:gate',
  CLAUDE_PORT: 'claude:port',       // push (main → renderer, MessagePort)

  // ── Claude session store (snapshots) ──
  CLAUDE_SESSION_GET: 'claude-session:get',
  CLAUDE_SESSION_GET_ALL: 'claude-session:getAll',
  CLAUDE_SESSION_UPDATE: 'claude-session:update', // push (main → renderer)

  // ── Claude session discovery ──
  CLAUDE_SESSIONS_LIST_FOR_DIR: 'claude-sessions:listForDir',

  // ── Claude profiles ──
  CLAUDE_PROFILES_LIST: 'claude-profiles:list',
  CLAUDE_PROFILES_ADD: 'claude-profiles:add',
  CLAUDE_PROFILES_UPDATE: 'claude-profiles:update',
  CLAUDE_PROFILES_REMOVE: 'claude-profiles:remove',

  // ── Hub ──
  HUB_LIST_PLUGINS: 'hub:listPlugins',
  HUB_PUBLISH: 'hub:publish',
  HUB_GET_STATUS: 'hub:getStatus',
  HUB_GET_REMOTE_INFO: 'hub:getRemoteInfo',
  HUB_INSTALL_PLUGIN: 'hub:installPlugin',
  HUB_REMOVE_PLUGIN: 'hub:removePlugin',
  HUB_EVENT: 'hub:event',           // push (main → renderer)
  HUB_STATUS: 'hub:status',         // push (main → renderer)

  // ── Shared layout document (hub-owned; tmux-style mirror) ──
  LAYOUT_GET: 'layout:get',
  LAYOUT_SET: 'layout:set',
  LAYOUT_CHANGED: 'layout:changed', // push (main → renderer)

  // ── Config ──
  CONFIG_GET: 'config:get',
  CONFIG_RELOAD: 'config:reload',
  CONFIG_GET_PATH: 'config:getPath',
  CONFIG_SAVE: 'config:save',

  // ── Session persistence ──
  SESSION_LIST: 'session:list',
  SESSION_LOAD: 'session:load',
  SESSION_SAVE: 'session:save',
  SESSION_DELETE: 'session:delete',

  // ── Analytics ──
  ANALYTICS_SUMMARY: 'analytics:summary',
  ANALYTICS_RECENT: 'analytics:recent',

  // ── Layout templates ──
  LAYOUTS_LIST: 'layouts:list',
  LAYOUTS_SAVE: 'layouts:save',
  LAYOUTS_DELETE: 'layouts:delete',

  // ── App / dialog ──
  APP_GET_CWD: 'app:getCwd',
  DIALOG_PICK_FOLDER: 'dialog:pickFolder',
  DIALOG_PICK_FILES: 'dialog:pickFiles',

  // ── Browser cookies ──
  CHROME_COOKIES_IMPORT: 'chrome-cookies:import',

  // ── App lifecycle ──
  APP_BEFORE_QUIT: 'app:before-quit', // push (main → renderer)
} as const;

export type IpcChannelKey = keyof typeof IPC;
export type IpcChannelValue = typeof IPC[IpcChannelKey];
