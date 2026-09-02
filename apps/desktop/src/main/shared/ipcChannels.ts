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
  PROJECT_DOWNLOAD_ICON: 'project:downloadIcon',
  LIBRARY_LIST: 'library:list',
  LIBRARY_SAVE: 'library:save',
  LIBRARY_REMOVE: 'library:remove',
  LIBRARY_CHANGED: 'library:changed', // push (main → renderer)

  // ── Git worktrees (agent isolation) ──
  WORKTREE_INFO: 'worktree:info', // invoke: is cwd a repo (root/branch)?
  WORKTREE_CREATE: 'worktree:create', // invoke: create an agent worktree
  WORKTREE_REMOVE: 'worktree:remove', // invoke: tear down an agent worktree (guarded)

  // ── In-app updates (electron-updater) ──
  UPDATES_STATUS_GET: 'updates:status-get', // invoke (renderer → main): current status
  UPDATES_CHECK: 'updates:check', // invoke (renderer → main): manual check now
  UPDATES_INSTALL: 'updates:install', // invoke (renderer → main): restart into a downloaded update
  UPDATES_STATUS: 'updates:status', // push (main → renderer): status transitions

  // ── Notifications / ambient awareness ──
  NOTIFY_SET_ACTIVE_SESSION: 'notify:set-active-session', // send (renderer → main)
  NOTIFY_FOCUS_AGENT: 'notify:focus-agent', // push (main → renderer)
  NOTIFY_IN_APP: 'notify:in-app', // push (main → renderer): notification-center entry
  NOTIFY_ESCALATE: 'notify:escalate', // send (renderer → main): raise OS notification for an unfocused window
  NOTIFY_ACTIVATE: 'notify:activate', // push (main → renderer): escalated notification was clicked
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
  // A facade caller (an agent's `open_terminal` → terminals.open) asked to open
  // a VISIBLE terminal pane running a command — push it to the renderer to open.
  FACADE_OPEN_TERMINAL: 'terminal:facade-open', // push (main → renderer)

  // ── Claude sessions ──
  CLAUDE_SPAWN: 'claude:spawn',
  CONFIG_CHANGED: 'config:changed', // push (main → renderer)
  CLAUDE_LIST_MODELS: 'claude:listModels',
  AGENT_SUGGEST_TITLE: 'agent:suggestTitle',
  WORKFLOW_AGENT_TRANSCRIPT: 'workflow:agentTranscript',
  WORKFLOW_AGENT_CONVERSATION: 'workflow:agentConversation',
  PROVIDER_LIST_MODELS: 'provider:listModels',
  PROVIDER_CHECK_ALL: 'provider:checkAll',
  KEEPWARM_HEARTBEATS: 'keepwarm:heartbeats',
  USAGE_REPORT: 'usage:report',
  CLAUDE_MESSAGE: 'claude:message',
  CLAUDE_SET_PERMISSION_MODE: 'claude:setPermissionMode',
  CLAUDE_SET_EFFORT: 'claude:setEffort',
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
  CLAUDE_SESSIONS_RECENT: 'claude-sessions:recent',
  CLAUDE_SESSIONS_LIVE_IDS: 'claude-sessions:liveIds',

  // ── Claude profiles ──
  CLAUDE_PROFILES_LIST: 'claude-profiles:list',
  CLAUDE_PROFILES_ADD: 'claude-profiles:add',
  CLAUDE_PROFILES_ADD_ACCOUNT: 'claude-profiles:addAccount',
  CLAUDE_PROFILES_LOGIN_STATUS: 'claude-profiles:loginStatus',
  /** Per-profile account identity read from each harness's own credential file
   *  (config root, stable account id, auth mode). Desktop-only. */
  CLAUDE_PROFILES_ACCOUNTS: 'claude-profiles:accounts',
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
  HUB_SESSION_GRANT_RECONCILE: 'hub:sessionGrantReconcile', // invoke: re-align a live manager/supervisor session token's full-access grant with config
  HUB_SET_REMOTE_SERVER: 'hub:setRemoteServer', // invoke: persist/clear the "connect to remote server" target
  TAILSCALE_GET_INFO: 'tailscale:getInfo',
  TAILSCALE_SET_SERVE: 'tailscale:setServe',
  HUB_INSTALL_PLUGIN: 'hub:installPlugin',
  HUB_INSPECT_PLUGIN: 'hub:inspectPlugin',
  HUB_CHECK_PLUGIN_UPDATES: 'hub:checkPluginUpdates',
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

  // ── Federation (peer hubs) ──
  // Peer list snapshot. Live transitions ride the generic HUB_EVENT feed as
  // hub.peer.connected / hub.peer.disconnected — re-invoke this on those.
  FEDERATION_PEERS: 'federation:peers',

  // ── Remote worker nodes (the hub's node registry) ──
  /** `nodes.list`, or null when this hub has NO registry — which is every
   *  ordinary install. The absence is read off the router's "no provider"
   *  error, not off a permission check: `nodes.list` is in the view tier
   *  unconditionally, so `can()` says yes on a hub that has no nodes at all. */
  NODES_LIST: 'nodes:list',
  /** `nodes.wake` — starts a billable machine. Host authority only on the hub;
   *  main's connection presents the host token, so the desktop always may.
   *  Live state follows on the generic HUB_EVENT feed as node.state_changed. */
  NODES_WAKE: 'nodes:wake',
  /** `nodes.sleep` — stops a running machine. Host authority only on the hub,
   *  same gate as the wake and for a reason of its own: a stop ends the work in
   *  flight on a machine somebody may be using. Live state follows on the
   *  generic HUB_EVENT feed as node.state_changed. */
  NODES_SLEEP: 'nodes:sleep',

  // ── Hub jobs (passthroughs to the hub's trusted-only jobs.* RPCs) ──
  JOBS_LIST: 'jobs:list',
  JOBS_UPSERT: 'jobs:upsert',
  JOBS_REMOVE: 'jobs:remove',
  JOBS_RUN: 'jobs:run',
  JOBS_HISTORY: 'jobs:history',
  /** Fetch a REMOTE session's conversation over its federation link (handler
   *  lives in federationBridge.ts). */
  FEDERATION_CONVERSATION: 'federation:conversation',
  /** Read / write ~/.config/workspacer/peers.json (handlers in
   *  federationPeersConfig.ts); a save restarts the hub so it takes effect. */
  FEDERATION_PEERS_CONFIG: 'federation:peersConfig',
  FEDERATION_SAVE_PEERS_CONFIG: 'federation:savePeersConfig',

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
  FONTS_INSTALL_CUSTOM: 'fonts:installCustom', // invoke: pick + install a UI font file
  FONTS_LIST_CUSTOM: 'fonts:listCustom', // invoke: installed custom UI fonts

  // ── Files (editor pane) ──
  FILE_READ: 'file:read',
  FILE_READ_IMAGE: 'file:read-image', // invoke: thumbnail data URL for an image path
  CLIPBOARD_SAVE_IMAGE: 'clipboard:save-image', // invoke: spill a pasted image to a temp PNG
  FILE_WRITE: 'file:write',
  FILE_LIST_DIR: 'file:listDir',
  FILE_WATCH: 'file:watch',
  FILE_UNWATCH: 'file:unwatch',
  FILE_CHANGED: 'file:changed', // push (main → renderer)
  FILE_OPEN_EXTERNAL: 'file:open-external', // invoke: open file:// URL in the OS default app/browser
  FILE_SHOW_IN_FOLDER: 'file:show-in-folder', // invoke: reveal in the OS file manager
  SHELL_OPEN_EXTERNAL: 'shell:open-external', // invoke: open an http(s) URL in the OS default browser

  // ── Brief board (BoardPane) ──
  BRIEF_BOARD_LOAD: 'brief-board:load', // invoke: every project's brief as cards
  BRIEF_BOARD_MOVE: 'brief-board:move', // invoke: drag a card to a column (or Archive)

  // ── Project search (editor search sidebar) ──
  SEARCH_PROJECT: 'search:project',

  // ── External tool availability (git / provider CLIs / tailscale) ──
  TOOLS_STATUS: 'tools:status',

  // ── Git (review pane) ──
  GIT_STATUS: 'git:status',
  GIT_LOG: 'git:log',
  GIT_COMMIT_DIFF: 'git:commitDiff',
  GIT_COMMIT_NUMSTAT: 'git:commitNumstat',
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
