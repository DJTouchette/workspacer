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

  // ── Git worktrees (agent isolation) ──
  WORKTREE_INFO: 'worktree:info', // invoke: is cwd a repo (root/branch)?
  WORKTREE_CREATE: 'worktree:create', // invoke: create an agent worktree

  // ── In-app updates (electron-updater) ──
  UPDATES_STATUS_GET: 'updates:status-get', // invoke (renderer → main): current status
  UPDATES_CHECK: 'updates:check', // invoke (renderer → main): manual check now
  UPDATES_INSTALL: 'updates:install', // invoke (renderer → main): restart into a downloaded update
  UPDATES_STATUS: 'updates:status', // push (main → renderer): status transitions

  // ── Notifications / ambient awareness ──
  NOTIFY_SET_ACTIVE_SESSION: 'notify:set-active-session', // send (renderer → main)
  NOTIFY_FOCUS_AGENT: 'notify:focus-agent', // push (main → renderer)
  SYSTEM_NOTICE: 'system:notice', // push (main → renderer): daemon/startup failures etc.
  LOGS_OPEN_FOLDER: 'logs:openFolder', // invoke (renderer → main): reveal the logs dir

  // ── Bundled workspacer CLI ──
  CLI_INSTALL: 'cli:install', // invoke (renderer → main): put the bundled CLI on PATH

  // ── Model pricing overrides (~/.workspacer/model-rates.json) ──
  PRICING_GET: 'pricing:get', // invoke: { defaults, overrides } for the Settings editor
  PRICING_SAVE: 'pricing:save', // invoke: persist the overrides map

  // ── Generic terminal ──
  TERMINAL_CREATE: 'terminal:create',
  TERMINAL_RESIZE: 'terminal:resize',
  TERMINAL_CLOSE: 'terminal:close',
  TERMINAL_PORT: 'terminal:port', // push (main → renderer, MessagePort)
  TERMINAL_EXIT: 'terminal:exit', // push (main → renderer)

  // ── Claude sessions ──
  CLAUDE_SPAWN: 'claude:spawn',
  CLAUDE_LIST_MODELS: 'claude:listModels',
  WORKFLOW_AGENT_TRANSCRIPT: 'workflow:agentTranscript',
  WORKFLOW_AGENT_CONVERSATION: 'workflow:agentConversation',
  PROVIDER_LIST_MODELS: 'provider:listModels',
  PROVIDER_CHECK_ALL: 'provider:checkAll',
  CLAUDE_MESSAGE: 'claude:message',
  CLAUDE_SET_PERMISSION_MODE: 'claude:setPermissionMode',
  CLAUDE_SET_MODEL: 'claude:setModel',
  CLAUDE_HANDOFF_BRIEF: 'claude:handoffBrief',
  CLAUDE_HANDOFF_AGENT_BRIEF: 'claude:handoffAgentBrief',
  CLAUDE_APPROVE: 'claude:approve',
  CLAUDE_ANSWER: 'claude:answer',
  CLAUDE_RESIZE: 'claude:resize',
  CLAUDE_SIGNAL: 'claude:signal',
  CLAUDE_CLOSE: 'claude:close',
  CLAUDE_ATTACH: 'claude:attach',
  CLAUDE_DETACH: 'claude:detach',
  CLAUDE_GATE: 'claude:gate',
  CLAUDE_PORT: 'claude:port', // push (main → renderer, MessagePort)

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
  HUB_SET_REMOTE_SHARE: 'hub:setRemoteShare',
  HUB_REMOTE_TOKENS_LIST: 'hub:remoteTokensList',
  HUB_REMOTE_TOKEN_GET_OR_CREATE: 'hub:remoteTokenGetOrCreate',
  HUB_REMOTE_TOKEN_REVOKE: 'hub:remoteTokenRevoke',
  HUB_SET_REMOTE_SERVER: 'hub:setRemoteServer', // invoke: persist/clear the "connect to remote server" target
  TAILSCALE_GET_INFO: 'tailscale:getInfo',
  TAILSCALE_SET_SERVE: 'tailscale:setServe',
  HUB_INSTALL_PLUGIN: 'hub:installPlugin',
  HUB_INSPECT_PLUGIN: 'hub:inspectPlugin',
  HUB_LIST_EXAMPLES: 'hub:listExamples',
  HUB_INSTALL_EXAMPLE: 'hub:installExample',
  HUB_REMOVE_PLUGIN: 'hub:removePlugin',
  HUB_SET_PLUGIN_ENABLED: 'hub:setPluginEnabled',
  HUB_PLUGIN_PANE_TOKEN: 'hub:pluginPaneToken',
  HUB_PLUGIN_PANE_TOKEN_REVOKE: 'hub:pluginPaneTokenRevoke',
  HUB_PLUGIN_SETTINGS_GET: 'hub:pluginSettingsGet',
  HUB_PLUGIN_SETTINGS_SET: 'hub:pluginSettingsSet',
  HUB_PLUGIN_SETTINGS_CHANGED: 'hub:pluginSettingsChanged', // push (main → renderer)
  HUB_EVENT: 'hub:event', // push (main → renderer)
  HUB_STATUS: 'hub:status', // push (main → renderer)

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
  APP_SUPERVISOR_HOME: 'app:supervisorHome',
  DIALOG_PICK_FOLDER: 'dialog:pickFolder',
  DIALOG_PICK_FILES: 'dialog:pickFiles',

  // ── Files (editor pane) ──
  FILE_READ: 'file:read',
  FILE_WRITE: 'file:write',
  FILE_LIST_DIR: 'file:listDir',
  FILE_WATCH: 'file:watch',
  FILE_UNWATCH: 'file:unwatch',
  FILE_CHANGED: 'file:changed', // push (main → renderer)
  FILE_OPEN_EXTERNAL: 'file:open-external', // invoke: open file:// URL in the OS default app/browser
  FILE_SHOW_IN_FOLDER: 'file:show-in-folder', // invoke: reveal in the OS file manager
  SHELL_OPEN_EXTERNAL: 'shell:open-external', // invoke: open an http(s) URL in the OS default browser

  // ── Project search (editor search sidebar) ──
  SEARCH_PROJECT: 'search:project',

  // ── External tool availability (git / provider CLIs / tailscale) ──
  TOOLS_STATUS: 'tools:status',

  // ── Git (review pane) ──
  GIT_STATUS: 'git:status',
  GIT_LOG: 'git:log',
  GIT_DIFF: 'git:diff',
  GIT_NUMSTAT: 'git:numstat',
  GIT_STAGE: 'git:stage',
  GIT_UNSTAGE: 'git:unstage',
  GIT_COMMIT: 'git:commit',
  GIT_PUSH: 'git:push',

  // ── Browser cookies ──
  CHROME_COOKIES_IMPORT: 'chrome-cookies:import',

  // ── App lifecycle ──
  APP_BEFORE_QUIT: 'app:before-quit', // push (main → renderer)
  APP_QUIT_SAVED: 'app:quit-saved', // ack (renderer → main): the quit-save landed
  APP_RELAUNCH: 'app:relaunch', // invoke: relaunch the app (applies remote-client connect/disconnect)

  // ── Window chrome (Windows native caption-button overlay) ──
  WINDOW_SET_OVERLAY: 'window:setOverlay', // send (renderer → main)
} as const;

export type IpcChannelKey = keyof typeof IPC;
export type IpcChannelValue = (typeof IPC)[IpcChannelKey];
