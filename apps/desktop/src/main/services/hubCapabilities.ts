/**
 * Real capabilities the main process exposes on the hub bus. These are the
 * inverse of events — things a plugin (or, later, Claude via the MCP facade)
 * can *ask workspacer to do*. Kept small and explicit; each is a future MCP tool.
 */

import { Notification, shell } from 'electron';
import * as os from 'os';
import * as fs from 'fs';
import * as path from 'path';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { applyLiveEffort } from './liveEffort';
import { agentHandoffBrief } from './agentHandoff';
import { spawnManagedAgent } from './managedSpawn';
import { spawnClaudeAgent } from './claudeSpawn';
import { resolveAgentBinary, checkAllProviders, type AgentProvider } from './agentProviders';
import { claudeProfiles } from './claudeProfiles';
import { registerCapability, callHub } from './hubClient';
import { agentNotifier } from './agentNotifier';
import { appIconPath } from '../lib/appIcon';
import { dropHostTrusted } from '../lib/hostTrustedConfig';
import { assertPathAllowed, canonicalRoot, configStoreRoots } from '../lib/pathConfinement';
import { DELEGATE_CATALOG_TO_BRAIN } from './brainDelegation';
import { configService } from './configService';
import { listClaudeModels } from './claudeModels';
import { libraryService } from './libraryService';
import { sessionService } from './sessionService';
import { sessionHistory } from './sessionHistory';
import { layoutService } from './layoutService';
import { listClaudeSessionsForDir } from './claudeSessionList';
import { listRecentSessions } from './recentSessions';
import { timelineReplay, type ReplayOp } from './timelineReplayService';
import { readTextFile, writeTextFile, listDir } from './fileService';
import { readImagePreview } from './imagePreview';
import { startWatch, stopWatch } from './fileWatchService';
import { searchProject } from './searchService';
import * as git from './gitService';
import * as terminalShare from './terminalShare';
import { IPC } from '../shared/ipcChannels';
import type { SessionData, LayoutInput, ProfileUpdate } from '../shared/ipcTypes';
import { ensureSupervisorHome } from './supervisorSkill';

// Mirror of ipc.ts's shell detection so a capability-spawned terminal picks the
// same default shell a UI-spawned one would. Kept local to avoid importing the
// IPC module (which pulls in Electron BrowserWindow plumbing).
function detectDefaultShell(): string {
  if (process.platform === 'win32') {
    const gitBash = 'C:\\Program Files\\Git\\bin\\bash.exe';
    try {
      fs.accessSync(gitBash);
      return gitBash;
    } catch {}
    try {
      require('child_process').execSync('where pwsh.exe', { stdio: 'ignore' });
      return 'pwsh.exe';
    } catch {}
    return 'powershell.exe';
  }
  return process.env.SHELL || '/bin/sh';
}

// ── Filesystem path confinement for fs.* / search.project ──────────────────
//
// These capabilities run in the trusted main process and, under remote sharing,
// are reachable by a web/phone client holding the shared host token — which the
// hub classifies as `trusted`, so its per-plugin path confinement does NOT apply.
// Left open, a remote caller could `fs.read('/etc/passwd')` or
// `fs.write('~/.ssh/authorized_keys')`. The desktop renderer never uses these bus
// capabilities (it edits over the `file:*` / `search:*` IPC path instead), so
// every bus call that reaches them is an external caller (web / remote / MCP, or a
// plugin the hub already confined to its grant). We therefore confine them here to
// the directories the web workspace legitimately touches:
//
//   - each live agent's cwd — the workspaces the editor / search / watch act on
//   - the three config-dir subtrees the UI actually edits (library/, layouts/,
//     sessions/) — NOT the config dir as a whole. The config dir is where
//     remote-token, tokens.json, remote-server.json, vapid.json and every
//     installed plugin's .bus-token / plaintext .settings.json live, so a root
//     that spans it hands any caller the credential that would promote it to a
//     `trusted` bus connection. Those subtrees are all the file-level access the
//     web client ever needed; everything else in the config dir is reached
//     through a typed capability (config.get/save, layouts.*, library.*), never
//     through fs.*.
//
// The directory *picker* (fs.listDir) additionally allows browsing the home tree,
// since its whole job is choosing a not-yet-open working directory for a new agent
// (it only lists non-hidden directory names, never file contents). Note this also
// intersects a plugin's own fs grant with these roots; a plugin needing fs access
// to a root outside the workspace would need that root added here (or a per-caller
// identity seam) — acceptable today since plugin fs grants target project files,
// which are agent cwds. The one grant this narrowing takes away is a `${pluginDir}`
// fs scope (that dir is under the config dir): no catalog plugin declares one, and
// a sidecar reads its own directory with local Node fs rather than over the bus.

// The predicate itself now lives in ../lib/pathConfinement (canonicalizePath,
// canonicalRoot, isWithin, pathWithinRoots, configStoreRoots, SECRET_BASENAMES,
// isSecretPath, assertPathAllowed) so the cross-language contract test can pin it
// directly instead of through a capability handler. What stays here is the root
// SUPPLY — which allow-list each capability gets — because that depends on the
// live session store.

/** Workspace roots for content-touching fs.* calls: live agent cwds + config stores. */
function workspaceRoots(): string[] {
  const roots = new Set<string>();
  for (const s of claudeSessionStore.getAllSnapshots()) {
    if (s.cwd) roots.add(s.cwd);
  }
  for (const r of configStoreRoots()) roots.add(r);
  return [...roots];
}

/** Broader roots for the directory picker: the home tree plus the workspace roots. */
function browseRoots(): string[] {
  return [os.homedir(), ...workspaceRoots()];
}

/**
 * Open a URL with the OS default handler, refusing any scheme but http(s).
 * `shell.openExternal` will happily launch a `file://` path or hand a custom
 * protocol to whatever app claims it, so every caller — the IPC handler the
 * renderer uses and the notification-click sink below — has to go through one
 * check rather than each remembering to write its own. Exported because this is
 * the only file in this lane both sinks can share; ipc.ts's SHELL_OPEN_EXTERNAL
 * handler has the identical result shape and should call this.
 */
export async function openExternalUrl(url: string): Promise<{ ok: boolean; error?: string }> {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { ok: false, error: 'Invalid URL' };
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    return { ok: false, error: `Refusing to open ${parsed.protocol} URL` };
  }
  try {
    await shell.openExternal(parsed.toString());
    return { ok: true };
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : String(err) };
  }
}

export function registerHubCapabilities(): void {
  // Adopted-hub note (adopt-don't-kill, see hubDaemon.ts): when the app adopts
  // a `workspacer serve` hub, its full-scope brain already provides most of
  // this surface. We still register everything below UNCHANGED, on purpose:
  // the hub router is first-registration-wins (services/hub internal/bus/
  // rpc.go), so brain-owned methods are simply withheld from us while the
  // methods the brain doesn't provide headlessly (analytics.*, fs.watch/
  // unwatch, OS notifications.post, terminal share) register fine — partial
  // registration is native to the bus, no pre-negotiation needed. Whichever
  // side owns a method serves it against the same claudemon, so the overlap
  // is harmless; hubClient logs the withheld set from the `registered` ack.
  // (Known minor degradation: an adopted brain owns notifications.post and
  // only logs it — plugin notifications won't raise OS toasts in that mode.)

  // `cat` registers a file-backed "catalog" capability — but no-ops when we
  // delegate the catalog to the headless brain provider (the hub spawns it with
  // --brain-scope catalog). The bus router is single-owner per method, so main
  // must not also register these or the two providers would collide. The
  // live/enriched agent + streaming caps below keep using registerCapability —
  // main still owns those. See brainDelegation.ts.
  const cat: typeof registerCapability = DELEGATE_CATALOG_TO_BRAIN ? () => {} : registerCapability;

  // Read-only: list live agents with light state. The bread-and-butter "what's
  // running?" call for any dashboard plugin or MCP client.
  registerCapability('agents.list', () =>
    claudeSessionStore.getAllSnapshots().map((s) => ({
      sessionId: s.sessionId,
      cwd: s.cwd,
      state: s.ambientState,
      model: s.usage?.model ?? null,
      contextTokens: s.usage?.contextTokens ?? 0,
      contextLimit: s.usage?.contextLimit ?? 0,
      costUSD: s.usage?.costUSD ?? 0,
      // What the agent is blocked on, if anything — lets a remote client show
      // the actual approval/question instead of a generic "waiting" badge.
      pendingApproval: s.pendingApproval
        ? { toolName: s.pendingApproval.toolName, toolInput: s.pendingApproval.toolInput }
        : null,
      pendingQuestions: s.pendingQuestions ?? null,
    })),
  );

  // Control: send a prompt to an agent. claudemon's /message owns the whole
  // delivery policy: it queues while the agent is busy (or a dialog is up),
  // injects once the prompt has settled, and verifies the submit took. A 409
  // now only means the session has ended — raw PTY typing can't help there
  // (and the old fallback could press Enter on an open permission dialog), so
  // surface the rejection to the caller instead.
  registerCapability('agents.sendMessage', async (params: unknown) => {
    const { sessionId, text } = (params ?? {}) as { sessionId?: string; text?: string };
    if (!sessionId || typeof text !== 'string') {
      throw new Error('agents.sendMessage requires { sessionId, text }');
    }
    const res = await claudemonSessionClient.message(sessionId, text);
    if (!res.ok) {
      throw new Error(`session is not accepting input (mode=${res.mode ?? 'unknown'})`);
    }
    return { ok: true };
  });

  // Control: spawn a brand-new Claude Code agent session. The hub/MCP
  // counterpart of the `claude:spawn` IPC handler — lets a remote client (or
  // Claude via the MCP facade) start a fresh agent in a directory. Shares the
  // spawn body with the IPC path via spawnClaudeAgent (see claudeSpawn.ts) so
  // the two transports stay identical — including per-spawn Library MCP servers
  // (`mcpItemIds`), which this path used to silently drop. Returns the new
  // sessionId so the caller can immediately drive it with the other
  // capabilities. The session runs headless in claudemon; a desktop pane can
  // attach to it later via the normal attach flow.
  registerCapability('agents.spawn', async (params: unknown) => {
    const {
      provider,
      transport: reqTransport,
      cwd,
      profileId,
      model,
      effort,
      permissionMode: reqMode,
      skipPermissions: reqSkip,
      resumeSessionId,
      cols,
      rows,
      supervisor,
      mcpFacade,
      label,
      parentSessionId,
      mcpItemIds,
    } = (params ?? {}) as {
      provider?: AgentProvider;
      /** Claude only: 'pty' | 'stream'. Omitted = the config default. */
      transport?: 'pty' | 'stream';
      cwd?: string;
      profileId?: string;
      model?: string;
      effort?: string;
      permissionMode?: string;
      skipPermissions?: boolean;
      resumeSessionId?: string;
      cols?: number;
      rows?: number;
      supervisor?: boolean;
      mcpFacade?: boolean;
      label?: string;
      parentSessionId?: string;
      mcpItemIds?: string[];
    };
    // SECURITY: this capability is the REMOTE/web/MCP spawn path (the local
    // desktop spawns over IPC). Driving an agent is already code execution on
    // the host, but we refuse to let a remote caller silently auto-bypass every
    // approval (`--dangerously-skip-permissions` / bypass-sandbox). Approvals
    // still surface and can be answered remotely; a YOLO agent must be started
    // locally. So `skipPermissions` is forced off here.
    if (reqSkip || reqMode === 'bypassPermissions' || reqMode === 'yolo') {
      console.warn(
        '[hub] agents.spawn: ignoring permission bypass from a bus client — remote spawns never auto-bypass approvals.',
      );
    }
    const skipPermissions = false;
    // …and the same for a bypass smuggled in through the PROFILE: clamping the
    // request's own fields left `profileId` as an open door (a bus caller can
    // create a profile with `--dangerously-skip-permissions` in extraArgs, or
    // reuse the user's own YOLO profile). The brain already scrubbed this; the
    // desktop path did not, so the two stacks disagreed on the invariant.
    const scrubProfileBypass = true;
    const permissionMode =
      reqMode === 'bypassPermissions' || reqMode === 'yolo' ? undefined : reqMode;
    // Managed (Tier-2) backend — Codex / OpenCode / Pi run through claudemon's
    // adapter, not a Claude PTY. Shares the dispatch with the `claude:spawn` IPC
    // handler so this path can't silently fall back to spawning Claude (it did
    // before — `provider` was ignored here, which is why a Codex agent spawned
    // from the web/remote client came up as Claude).
    if (provider && provider !== 'claude') {
      const sessionId = await spawnManagedAgent({
        provider,
        cwd,
        // Codex mirrors Claude's stream transport: 'stream' spawns headless
        // (GUI-only, daemon-owned thread). Must ride through here like on the
        // IPC branch (ipc.ts) or a remote headless spawn silently downgrades
        // to the hybrid PTY session.
        ...(provider === 'codex' && reqTransport === 'stream' && { transport: 'stream' as const }),
        model,
        effort,
        skipPermissions,
        resumeSessionId,
        supervisor,
        mcpFacade,
        label,
        parentSessionId,
        cols,
        rows,
      });
      return { sessionId };
    }
    // Claude on the 'stream' transport is managed too (claudemon's headless
    // stream-json adapter, no PTY) — same shared dispatch as the IPC path so
    // the two spawn transports can't drift (standing project rule).
    const transport = reqTransport ?? configService.getConfig().claude?.transport ?? 'pty';
    if (transport === 'stream') {
      const sessionId = await spawnManagedAgent({
        provider: 'claude',
        transport: 'stream',
        cwd,
        // Profile + per-spawn Library MCP servers must ride through here just
        // like on the IPC stream branch (ipc.ts) — this path used to drop both,
        // so a remote stream spawn silently ignored the chosen profile/servers.
        profileId,
        model,
        permissionMode,
        skipPermissions,
        resumeSessionId,
        supervisor,
        mcpFacade,
        label,
        parentSessionId,
        mcpItemIds,
        scrubProfileBypass,
      });
      return { sessionId };
    }
    const sessionId = await spawnClaudeAgent({
      cwd,
      profileId,
      scrubProfileBypass,
      model,
      permissionMode,
      skipPermissions,
      resumeSessionId,
      supervisor,
      mcpFacade,
      label,
      parentSessionId,
      cols,
      rows,
      mcpItemIds,
    });
    return { sessionId };
  });

  // Control: open a new shell terminal session. The hub/MCP counterpart of the
  // `terminal:create` IPC handler. Returns the new PTY's session id.
  registerCapability('terminals.create', async (params: unknown) => {
    const { shell, cwd, cols, rows } = (params ?? {}) as {
      shell?: string;
      cwd?: string;
      cols?: number;
      rows?: number;
    };
    const resolvedShell = shell || detectDefaultShell();
    const resolvedCwd = cwd && fs.existsSync(cwd) ? cwd : os.homedir();
    const id = await claudemonSessionClient.spawn({
      argv: [resolvedShell],
      cwd: resolvedCwd,
      cols,
      rows,
      portChannel: IPC.TERMINAL_PORT,
    });
    return { sessionId: id };
  });

  // Surface a notification: recorded in the in-app notification center, and —
  // unless `inAppOnly` or the user disabled notifications — also shown as a
  // clickable OS notification. Clicking focuses the target agent when
  // `sessionId` is given, opens `url` externally when given, else just brings
  // the window forward. `source` labels the sender in the center ("plugin:ci").
  registerCapability('notifications.post', (params: unknown) => {
    const p = (params ?? {}) as {
      title?: string;
      body?: string;
      level?: string;
      source?: string;
      sessionId?: string;
      paneType?: string;
      url?: string;
      key?: string;
      silent?: boolean;
      inAppOnly?: boolean;
    };
    const title = p.title || 'workspacer';
    const body = p.body || '';
    const level = (['info', 'success', 'warn', 'error'] as const).find((l) => l === p.level);

    agentNotifier.postInApp({
      level: level ?? 'info',
      title,
      body: body || undefined,
      source: typeof p.source === 'string' && p.source ? p.source : 'plugin',
      sessionId: p.sessionId,
      paneType: p.paneType,
      url: p.url,
      key: p.key,
      silent: p.silent === true,
    });

    const notifCfg =
      (configService.getConfig() as { notifications?: { enabled?: boolean; sound?: boolean } })
        .notifications ?? {};
    // `silent` means silent on every surface (history-only), matching the
    // renderer store and escalation path — not just "no toast".
    if (
      p.inAppOnly === true ||
      p.silent === true ||
      notifCfg.enabled === false ||
      !Notification.isSupported()
    ) {
      return { ok: true, os: false };
    }
    const notification = new Notification({
      title,
      body,
      icon: appIconPath() ?? undefined,
      silent: notifCfg.sound !== true,
    });
    notification.on('click', () => {
      if (p.sessionId) agentNotifier.focusAgent(p.sessionId);
      // The url comes from whoever posted the notification — a plugin, or a
      // remote client — so it gets the same scheme check as the renderer's own
      // open-external path rather than being handed straight to the OS.
      else if (p.url) void openExternalUrl(p.url);
      else agentNotifier.focusWindow();
    });
    notification.on('failed', (_e, err) =>
      console.warn(
        `[notifications.post] OS notification failed (in-app center still has it): ${err}`,
      ),
    );
    notification.show();
    return { ok: true, os: true };
  });

  // Control: resolve a parked permission prompt. The remote counterpart of the
  // `claude:approve` IPC handler — this is what lets a phone unblock an agent.
  registerCapability('claude.approve', async (params: unknown) => {
    const { sessionId, decision, reason } = (params ?? {}) as {
      sessionId?: string;
      decision?: 'yes' | 'no' | 'always';
      reason?: string;
    };
    if (!sessionId || (decision !== 'yes' && decision !== 'no' && decision !== 'always')) {
      throw new Error("claude.approve requires { sessionId, decision: 'yes'|'no'|'always' }");
    }
    await claudemonSessionClient.approve(sessionId, decision, reason);
    return { ok: true };
  });

  // Control: live permission-mode switch (no restart). Remote counterpart of
  // the `claude:setPermissionMode` IPC handler; claudemon drives and verifies
  // the switch, and the snapshot store is updated the same way so remote and
  // desktop pills stay in sync.
  registerCapability('claude.setPermissionMode', async (params: unknown) => {
    const { sessionId, mode } = (params ?? {}) as { sessionId?: string; mode?: string };
    if (!sessionId || typeof mode !== 'string' || !mode) {
      throw new Error('claude.setPermissionMode requires { sessionId, mode }');
    }
    const result = await claudemonSessionClient.setPermissionMode(sessionId, mode);
    if (result.ok && result.mode) claudeSessionStore.notePermissionMode(sessionId, result.mode);
    return result;
  });

  // Control: live reasoning-effort switch (no restart). Remote counterpart of
  // the `claude:setEffort` IPC handler — same shared body, so a remote pill and
  // the desktop pill drive the identical per-provider mechanism.
  registerCapability('claude.setEffort', async (params: unknown) => {
    const { sessionId, effort } = (params ?? {}) as { sessionId?: string; effort?: string };
    if (!sessionId || typeof effort !== 'string' || !effort) {
      throw new Error('claude.setEffort requires { sessionId, effort }');
    }
    return applyLiveEffort(sessionId, effort);
  });

  // Control: live model/effort switch for managed providers (no restart).
  // Remote counterpart of the `claude:setModel` IPC handler; confirmation
  // flows back through the status line, so no store note is needed.
  registerCapability('claude.setModel', async (params: unknown) => {
    const { sessionId, model, effort } = (params ?? {}) as {
      sessionId?: string;
      model?: string;
      effort?: string;
    };
    if (!sessionId || (!model && !effort)) {
      throw new Error('claude.setModel requires { sessionId, model and/or effort }');
    }
    return claudemonSessionClient.setModel(sessionId, model, effort);
  });

  // Control: build a cross-provider handoff brief (persisted under
  // ~/.workspacer/handoffs/). Remote counterpart of `claude:handoffBrief`.
  registerCapability('claude.handoffBrief', async (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('claude.handoffBrief requires { sessionId }');
    return claudemonSessionClient.handoffBrief(sessionId);
  });

  // Control: agent-authored handoff brief (source agent writes the file;
  // mechanical fallback on timeout). Long-running — resolves when the brief
  // file exists.
  registerCapability('claude.handoffAgentBrief', async (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('claude.handoffAgentBrief requires { sessionId }');
    return agentHandoffBrief(sessionId);
  });

  // Control: answer an AskUserQuestion picker. Mirrors the desktop ClaudePane
  // handleAnswer — drive the picker by typing into the PTY rather than the
  // mode-gated /answer endpoint, which requires mode=Question and races with
  // concurrent hook events. claude's TUI accepts the numeric option (or free
  // text) followed by Enter exactly like any other keystroke, so this lands
  // reliably whether the picker arrived via PreToolUse or mid-stream.
  registerCapability('claude.answer', async (params: unknown) => {
    const { sessionId, option, text, answers, answerKinds } = (params ?? {}) as {
      sessionId?: string;
      option?: number;
      text?: string;
      answers?: string[];
      answerKinds?: string[];
    };
    if (!sessionId) throw new Error('claude.answer requires { sessionId, ... }');
    if (option === undefined && text === undefined && answers === undefined) {
      throw new Error('claude.answer requires one of { option, text, answers }');
    }
    // Stream-transport sessions have no PTY: raw input can't answer them, so
    // route structurally through POST /answer (the daemon resolves the parked
    // AskUserQuestion over the adapter's control protocol). PTY sessions keep
    // the keystroke path — /answer requires mode=Question, which races hook
    // mode flips (same reasoning as ClaudePane's handleAnswer).
    if (claudeSessionStore.getSnapshot(sessionId)?.transport === 'stream') {
      await claudemonSessionClient.answer(sessionId, { option, text, answers, answerKinds });
    } else if (option !== undefined) {
      await claudemonSessionClient.input(sessionId, `${option}\r`);
    } else if (text !== undefined) {
      await claudemonSessionClient.input(sessionId, `${text}\r`);
    } else if (answers) {
      // PTY sessions answer by typing each answer as keystrokes — the picker
      // can't disambiguate a literal numeric free-text answer from an option
      // index at the keystroke level, so `answerKinds` doesn't apply here.
      for (const a of answers) await claudemonSessionClient.input(sessionId, `${a}\r`);
    }
    claudeSessionStore.clearPendingQuestions(sessionId);
    return { ok: true };
  });

  // Control: send a POSIX signal to a session (e.g. SIGTERM to stop a runaway
  // agent, SIGINT to interrupt). Mirrors the `claude:signal` IPC handler.
  registerCapability('claude.signal', async (params: unknown) => {
    const { sessionId, signal } = (params ?? {}) as { sessionId?: string; signal?: string };
    if (!sessionId || !signal) throw new Error('claude.signal requires { sessionId, signal }');
    await claudemonSessionClient.signal(sessionId, signal);
    return { ok: true };
  });

  // Read-only: fetch a session's transcript so a remote client can show the
  // context behind a pending approval/question before answering. An optional
  // cwd lets callers reach historical sessions the daemon isn't tracking
  // (resolved from the on-disk JSONL under ~/.claude/projects).
  registerCapability('sessions.transcript', async (params: unknown) => {
    const { sessionId, cwd } = (params ?? {}) as { sessionId?: string; cwd?: string };
    if (!sessionId) throw new Error('sessions.transcript requires { sessionId }');
    return claudemonSessionClient.getTranscript(sessionId, cwd);
  });

  // ── Timeline worktree replay ─────────────────────────────────────────
  // Materializes a session's file edits into a disposable git worktree so a
  // replay UI can scrub real files through time without touching the agent's
  // checkout. All writes are confined to worktrees the service itself creates
  // under the OS temp dir (see timelineReplayService).
  registerCapability('replay.open', async (params: unknown) => {
    const { cwd, sessionId, beforeTs } = (params ?? {}) as {
      cwd?: string;
      sessionId?: string;
      beforeTs?: string;
    };
    if (!cwd || !sessionId) throw new Error('replay.open requires { cwd, sessionId }');
    // `cwd` picks the REPOSITORY the replay worktree is cut from, so it needs
    // the same confinement every git.* handler gets (guardGitCwd). Without it a
    // plugin scoped to its own project could open a replay on any repo and read
    // files out of it through replay.read — bytes fs.read would have refused.
    // The canonical repo path is what the worktree is cut from — the checked
    // string and the used string are the same string.
    return timelineReplay.open(
      assertPathAllowed('replay.open', cwd, workspaceRoots()),
      sessionId,
      beforeTs,
    );
  });
  registerCapability('replay.seek', async (params: unknown) => {
    const { sessionId, ops } = (params ?? {}) as { sessionId?: string; ops?: ReplayOp[] };
    if (!sessionId) throw new Error('replay.seek requires { sessionId, ops }');
    return timelineReplay.seek(sessionId, Array.isArray(ops) ? ops : []);
  });
  registerCapability('replay.close', async (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('replay.close requires { sessionId }');
    return timelineReplay.close(sessionId);
  });
  // Reading back what a seek materialized. Without these, `replay.open`/`seek`
  // build a past nothing can look at: the worktree lives under the OS temp dir,
  // which is in neither a plugin token's fsRoots nor `workspaceRoots()`, so
  // fs.read / git.diff are (correctly) denied there.
  //
  // SECURITY: deliberately NOT in capspec's PathParam and deliberately not
  // guarded by assertPathAllowed. The `path` these take is interpreted inside a
  // worktree the replay service itself created and keyed by sessionId — it is a
  // repo-relative coordinate, not a host path, and the service refuses anything
  // that escapes (see resolveInside). Adding fsRoots scoping here would be
  // scoping the wrong namespace; the containment is structural.
  registerCapability('replay.read', async (params: unknown) => {
    const { sessionId, path: p } = (params ?? {}) as { sessionId?: string; path?: string };
    if (!sessionId || !p) throw new Error('replay.read requires { sessionId, path }');
    return timelineReplay.read(sessionId, p);
  });
  registerCapability('replay.diff', async (params: unknown) => {
    const { sessionId, path: p } = (params ?? {}) as { sessionId?: string; path?: string };
    if (!sessionId) throw new Error('replay.diff requires { sessionId }');
    return timelineReplay.diff(sessionId, p);
  });

  // Read-only: parsed conversation items + latest sequence number. With
  // sinceSeq, returns only items after that sequence — cheap incremental polling
  // so a supervisor digests just the new turns since it last looked.
  registerCapability('sessions.conversation', async (params: unknown) => {
    const { sessionId, sinceSeq } = (params ?? {}) as { sessionId?: string; sinceSeq?: number };
    if (!sessionId) throw new Error('sessions.conversation requires { sessionId }');
    return claudemonSessionClient.getConversation(
      sessionId,
      typeof sinceSeq === 'number' ? sinceSeq : undefined,
    );
  });

  // Live terminal mirror: a remote opening the terminal view attaches here,
  // which streams the session's raw PTY bytes onto the bus as
  // `pty.bytes.<sessionId>` events (see terminalShare). Keepalive holds the
  // lease open; detach (or a lapsed lease) stops the stream.
  registerCapability('sessions.attachTerminal', (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('sessions.attachTerminal requires { sessionId }');
    terminalShare.attachTerminal(sessionId);
    return { ok: true };
  });

  registerCapability('sessions.terminalKeepalive', (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('sessions.terminalKeepalive requires { sessionId }');
    return { ok: terminalShare.keepaliveTerminal(sessionId) };
  });

  registerCapability('sessions.detachTerminal', (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('sessions.detachTerminal requires { sessionId }');
    terminalShare.stopTerminal(sessionId);
    return { ok: true };
  });

  // Control: forward raw keystrokes from a remote terminal view into the PTY —
  // the write-side counterpart of the pty.bytes stream. Lets a phone actually
  // drive the terminal (type, Ctrl-C, answer raw prompts), not just watch it.
  registerCapability('sessions.terminalInput', async (params: unknown) => {
    const { sessionId, data } = (params ?? {}) as { sessionId?: string; data?: string };
    if (!sessionId || typeof data !== 'string') {
      throw new Error('sessions.terminalInput requires { sessionId, data }');
    }
    await claudemonSessionClient.input(sessionId, data);
    return { ok: true };
  });

  // Control: resize the session's PTY to the remote viewer's grid so wrapping
  // matches the phone's screen instead of the desktop pane. The PTY is shared,
  // so this reflows the desktop too — intentional: the active driver sets size.
  registerCapability('sessions.terminalResize', async (params: unknown) => {
    const { sessionId, cols, rows } = (params ?? {}) as {
      sessionId?: string;
      cols?: number;
      rows?: number;
    };
    if (!sessionId || !cols || !rows) {
      throw new Error('sessions.terminalResize requires { sessionId, cols, rows }');
    }
    await claudemonSessionClient.resize(sessionId, Math.round(cols), Math.round(rows));
    return { ok: true };
  });

  // ── Full session snapshots (web parity) ────────────────────────────────
  // The reduced `agents.list` row is enough for a dashboard badge; the web
  // renderer needs the *full* snapshot (transcript, tool calls, fleet/workflow
  // detail) that the desktop gets over the `claude-session:update` IPC. These
  // mirror the CLAUDE_SESSION_GET / GET_ALL handlers; live updates arrive as
  // `agent.snapshot` bus events (published from claudeSessionStore.pushUpdate).
  //
  // Shared fleet-visibility rule (services/hub cmd/brain/visibility.go): the
  // desktop sidebar also shows a *stopped* agent's card while it's curated in
  // the shared layout ("Stopped — respawn"), but the store only holds live
  // sessions (an ended one is evicted ~30s after SessionEnd). So the snapshot
  // list a bus client renders (/m fleet) appends minimal `sparse` stopped rows
  // synthesized from the hub's layout document — the same rows the brain
  // serves when it owns this method, keeping the two providers interchangeable.
  registerCapability('sessions.snapshots', async () => {
    const live = claudeSessionStore.getAllSnapshots();
    const liveIds = new Set(live.map((s) => s.sessionId));
    type LayoutAgentRef = {
      global?: boolean;
      sessionId?: string;
      lastSessionId?: string;
      cwd?: string;
      name?: string;
    };
    const stopped: unknown[] = [];
    try {
      const doc = await callHub<{ data?: { agents?: LayoutAgentRef[] } }>('layout.get');
      const seen = new Set<string>();
      for (const a of doc?.data?.agents ?? []) {
        if (!a || a.global) continue;
        const sid = a.sessionId || a.lastSessionId;
        if (!sid || liveIds.has(sid) || seen.has(sid)) continue;
        seen.add(sid);
        stopped.push({
          sessionId: sid,
          cwd: a.cwd ?? '',
          label: a.name,
          status: 'ended',
          ambientState: 'idle',
          pendingApproval: null,
          pendingQuestions: null,
          sparse: true,
        });
      }
    } catch {
      // No hub / no layout document — serve the live sessions only.
    }
    return [...live, ...stopped];
  });
  registerCapability('sessions.snapshot', (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('sessions.snapshot requires { sessionId }');
    return claudeSessionStore.getSnapshot(sessionId);
  });

  // The daemon's full session list — every resumable row (all providers,
  // including stopped and archived), enriched with the desktop's own history DB
  // (agent name, model, cost) and provider auto-titles. `sessions.snapshots`
  // above only covers LIVE sessions, so without this the web client's Sessions
  // pane and History row render empty.
  //
  // registerCapability, not `cat`: the enrichment joins main's session_history
  // SQLite and reads local transcripts for titles, so a headless brain can't
  // serve an equivalent — main must own this method on both delegation paths.
  registerCapability('sessions.recent', () => listRecentSessions());

  // ── Config (web parity) ────────────────────────────────────────────────
  // Mirror the CONFIG_* IPC handlers so the web renderer loads the real config
  // (theme, keybindings, pane settings) instead of falling back to defaults,
  // and can persist changes from the Settings pane.
  cat('config.get', () => configService.getConfig());
  cat('config.reload', () => configService.reloadConfig());
  cat('config.getPath', () => configService.getConfigPath());
  // Bus callers do not get to write host-trusted sections — see
  // lib/hostTrustedConfig for why `updates` in particular is code execution.
  // The brain applies the same drop; this is the copy that runs when catalog
  // delegation is off.
  cat('config.save', (params: unknown) =>
    configService.saveConfig(
      dropHostTrusted((params ?? {}) as Record<string, unknown>) as Parameters<
        typeof configService.saveConfig
      >[0],
    ),
  );

  // ── Model picker (web parity) ──────────────────────────────────────────
  cat('claude.listModels', () => listClaudeModels());

  // ── Provider discovery (web parity) ────────────────────────────────────
  // Mirror the PROVIDER_LIST_MODELS / PROVIDER_CHECK_ALL IPC handlers so the web
  // Spawn dialog can list a managed provider's models and show per-provider
  // detection dots, instead of falling back to a free-text model field. Both are
  // read-only discovery — no code execution beyond what the desktop IPC does
  // (listModels queries the provider's own CLI via claudemon; checkAll only
  // stats binaries on PATH), so they carry none of agents.spawn's bypass risk.
  registerCapability('providers.listModels', (params: unknown) => {
    const { provider, cwd } = (params ?? {}) as {
      provider?: 'codex' | 'opencode' | 'pi';
      cwd?: string;
    };
    if (provider !== 'codex' && provider !== 'opencode' && provider !== 'pi') {
      throw new Error("providers.listModels requires { provider: 'codex'|'opencode'|'pi' }");
    }
    const customBin = configService.getConfig().agents?.binaries?.[provider] ?? '';
    return claudemonSessionClient.listProviderModels(
      provider,
      cwd,
      resolveAgentBinary(provider, customBin),
    );
  });
  registerCapability('providers.checkAll', () => {
    const binaries = configService.getConfig().agents?.binaries ?? {};
    return checkAllProviders(binaries);
  });

  // ── Saved sessions (workspace layouts) ─────────────────────────────────
  // Mirror the SESSION_* IPC handlers so the web client can list/load/save the
  // saved agent arrangements (the session picker).
  cat('sessions.list', () => sessionService.listSessions());
  cat('sessions.load', (params: unknown) => {
    const { filename } = (params ?? {}) as { filename?: string };
    if (!filename) throw new Error('sessions.load requires { filename }');
    return sessionService.loadSession(filename);
  });
  cat('sessions.save', (params: unknown) => {
    const data = (params ?? {}) as SessionData;
    const ptyMapping = data.ptyMapping || {};
    if (Array.isArray(data.agents)) {
      return sessionService.saveSession({
        name: data.name,
        timestamp: new Date().toISOString(),
        activeAgentId: data.activeAgentId,
        agents: sessionService.enrichAgentsWithCwd(data.agents as any, ptyMapping),
      });
    }
    const enrichedTabs = (data.tabs || []).map((tab: any) => ({
      ...tab,
      panes: sessionService.enrichPanesWithCwd(tab.panes || [], ptyMapping),
    }));
    return sessionService.saveSession({
      name: data.name,
      timestamp: new Date().toISOString(),
      activeTabId: data.activeTabId,
      tabs: enrichedTabs,
    });
  });
  cat('sessions.delete', (params: unknown) => {
    const { filename } = (params ?? {}) as { filename?: string };
    if (!filename) throw new Error('sessions.delete requires { filename }');
    sessionService.deleteSession(filename);
    return { ok: true };
  });

  // ── Layout templates ───────────────────────────────────────────────────
  cat('layouts.list', () => layoutService.list());
  cat('layouts.save', (params: unknown) => layoutService.save((params ?? {}) as LayoutInput));
  cat('layouts.delete', (params: unknown) => {
    const { id } = (params ?? {}) as { id?: string };
    if (!id) throw new Error('layouts.delete requires { id }');
    layoutService.remove(id);
    return { ok: true };
  });

  // ── Claude profiles ────────────────────────────────────────────────────
  cat('claude.profiles.list', () => claudeProfiles.getProfiles());
  cat('claude.profiles.add', (params: unknown) => {
    const { name, configDir, extraArgs, mcpItemIds } = (params ?? {}) as {
      name?: string;
      configDir?: string;
      extraArgs?: string[];
      mcpItemIds?: string[];
    };
    if (!name) throw new Error('claude.profiles.add requires { name }');
    // Forward mcpItemIds — the web/remote client sends the user's selected MCP
    // servers here (matching the desktop IPC path); dropping it silently lost
    // them, so profiles created remotely had no MCP servers.
    return claudeProfiles.addProfile(name, configDir ?? '', extraArgs ?? [], mcpItemIds ?? []);
  });
  cat('claude.profiles.update', (params: unknown) => {
    const { id, updates } = (params ?? {}) as { id?: string; updates?: ProfileUpdate };
    if (!id) throw new Error('claude.profiles.update requires { id, updates }');
    return claudeProfiles.updateProfile(id, updates ?? ({} as ProfileUpdate));
  });
  cat('claude.profiles.remove', (params: unknown) => {
    const { id } = (params ?? {}) as { id?: string };
    if (!id) throw new Error('claude.profiles.remove requires { id }');
    claudeProfiles.removeProfile(id);
    return { ok: true };
  });

  // ── Claude session discovery (resume picker) ───────────────────────────
  cat('claude.sessionsForDir', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    if (!cwd) throw new Error('claude.sessionsForDir requires { cwd }');
    return listClaudeSessionsForDir(cwd);
  });

  // ── Library (reusable prompts + skills) ────────────────────────────────
  // The `cwd` these take picks the PROJECT whose .workspacer/library and
  // .claude/{skills,agents,commands} are listed, written and deleted. It reached
  // the service unchecked, so a bus caller could read a stranger's project assets
  // — and, worse, have library.save write markdown (or library.remove rm -rf a
  // skill dir) anywhere the desktop user can. Same confinement as the fs.* and
  // git.* handlers; the service itself stays unguarded because the local IPC path
  // is the trusted user working in their own repos.
  //
  // Each guard RETURNS the canonical cwd and the handler passes that on: the
  // string that was checked has to be the string the service opens, or a symlink
  // (or a `..` behind one) makes the check describe a different directory than
  // the read.
  const guardLibraryCwd = (cap: string, cwd?: string): string | undefined => {
    if (!cwd) return undefined;
    return assertPathAllowed(cap, cwd, workspaceRoots());
  };
  cat('library.list', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    // The read-only list gets browseRoots, not workspaceRoots, for the same
    // reason fs.listDir does: the New Agent dialog lists the library of the
    // directory the user is ABOUT to spawn in, which by definition isn't a live
    // agent cwd yet, and its `.catch(() => {})` would turn a refusal into a
    // silently empty project-MCP picker. Browsing the home tree to read a
    // project's own prompt files is the same exposure the picker already has;
    // writing and deleting stay on the workspace roots.
    const canonicalCwd = cwd ? assertPathAllowed('library.list', cwd, browseRoots()) : undefined;
    return libraryService.list(canonicalCwd);
  });
  cat('library.save', (params: unknown) => {
    const input = (params ?? {}) as { cwd?: string };
    const canonicalCwd = guardLibraryCwd('library.save', input.cwd);
    return libraryService.save({ ...input, cwd: canonicalCwd } as any);
  });
  cat('library.remove', (params: unknown) => {
    const { scope, id, cwd, kind } = (params ?? {}) as {
      scope?: 'global' | 'project' | 'claude';
      id?: string;
      cwd?: string;
      kind?: 'prompt' | 'skill' | 'agent';
    };
    if (!scope || !id) throw new Error('library.remove requires { scope, id }');
    const canonicalCwd = guardLibraryCwd('library.remove', cwd);
    libraryService.remove(scope, id, canonicalCwd, kind);
    return { ok: true };
  });

  // ── Analytics ──────────────────────────────────────────────────────────
  registerCapability('analytics.summary', (params: unknown) => {
    const { provider, since } = (params ?? {}) as { provider?: string; since?: string };
    return sessionHistory.summary(provider, since);
  });
  registerCapability('analytics.recent', (params: unknown) => {
    const { limit, provider, since } = (params ?? {}) as {
      limit?: number;
      provider?: string;
      since?: string;
    };
    return sessionHistory.recent(limit, provider, since);
  });

  // ── Approval gate + host cwd ───────────────────────────────────────────
  registerCapability('claude.gate', (params: unknown) => {
    const { sessionId, on } = (params ?? {}) as { sessionId?: string; on?: boolean };
    if (!sessionId) throw new Error('claude.gate requires { sessionId, on }');
    return claudemonSessionClient.setGate(sessionId, !!on);
  });
  registerCapability('app.getCwd', () => process.cwd());
  registerCapability('app.supervisorHome', () => ensureSupervisorHome());

  // ── Host filesystem browsing (web folder picker) ───────────────────────
  // The web client can't open a native OS dialog, so it browses the host's
  // directories through this to choose a working directory for a new agent.
  // Directories only (you spawn an agent *in* a folder); hidden entries skipped.
  cat('fs.listDir', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    const home = os.homedir();
    // A blank path is the picker's "start me somewhere" default, applied HERE as
    // an explicit handler default rather than inside the guard. What is NOT done
    // is tilde expansion: '~' is an ordinary filename to every layer that handles
    // a caller-supplied path (the brain used to expand it and this side did not,
    // so the same string was allowed by one provider and denied by the other).
    const requested = p && p.trim() ? p : home;
    // Browsing is limited to the home tree + live agent cwds so a remote client
    // can pick a project dir but can't enumerate /etc, /root, or other users' homes.
    // readdir gets the CANONICAL path the guard validated, never the raw request.
    const resolved = assertPathAllowed('fs.listDir', requested, browseRoots());
    let dirs: string[] = [];
    try {
      dirs = fs
        .readdirSync(resolved, { withFileTypes: true })
        .filter((e) => e.isDirectory() && !e.name.startsWith('.'))
        .map((e) => e.name)
        .sort((a, b) => a.localeCompare(b));
    } catch (err) {
      throw new Error(`cannot read ${resolved}: ${(err as Error).message}`);
    }
    return { path: resolved, parent: path.dirname(resolved), home, dirs };
  });

  // ── File read/write (editor pane) ──────────────────────────────────────
  // Same backend as the file:read/file:write IPC, so the web/phone client edits
  // the same host files as the desktop. Errors propagate as a failed call.
  cat('fs.read', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.read requires a path');
    return readTextFile(assertPathAllowed('fs.read', p, workspaceRoots()));
  });
  // Thumbnail for an image attachment — the web client's composer renders the
  // same previews as the desktop one, over host paths it can't read directly.
  //
  // registerCapability, NOT cat: the catalog fs.* methods (read/write/listEntries)
  // are delegated to the Go brain by default, and the brain has no fs.readImage
  // counterpart — registering this one through `cat` made it a no-op in the
  // default configuration, so every remote thumbnail failed with "no provider".
  // Main keeps it, the same way it keeps fs.watch/fs.unwatch.
  registerCapability('fs.readImage', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.readImage requires a path');
    return readImagePreview(assertPathAllowed('fs.readImage', p, workspaceRoots()));
  });
  cat('fs.write', (params: unknown) => {
    const { path: p, contents } = (params ?? {}) as { path?: string; contents?: string };
    if (!p) throw new Error('fs.write requires a path');
    return writeTextFile(assertPathAllowed('fs.write', p, workspaceRoots()), contents ?? '');
  });
  // Files-included, gitignore-aware listing for the editor's file tree (web client).
  cat('fs.listEntries', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.listEntries requires a path');
    return listDir(assertPathAllowed('fs.listEntries', p, workspaceRoots()));
  });

  // ── File watch (editor external-change detection, web client) ──────────
  // Starts/stops a host-side watch; the watcher's global emit sink (installed in
  // ipc.ts) mirrors every change onto the bus as a `fs.changed` event carrying
  // { path, eventType }, which webBackend subscribes to and filters by path.
  registerCapability('fs.watch', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.watch requires a path');
    startWatch(assertPathAllowed('fs.watch', p, workspaceRoots()));
    return { ok: true };
  });
  registerCapability('fs.unwatch', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.unwatch requires a path');
    stopWatch(assertPathAllowed('fs.unwatch', p, workspaceRoots()));
    return { ok: true };
  });

  // ── Project search (editor search sidebar, web client) ─────────────────
  // Same ripgrep backend as the search:project IPC.
  registerCapability('search.project', (params: unknown) => {
    const opts = (params ?? {}) as Parameters<typeof searchProject>[0];
    if (!opts.query) throw new Error('search.project requires { query, cwd }');
    if (!opts.cwd) throw new Error('search.project requires { query, cwd }');
    const cwd = assertPathAllowed('search.project', opts.cwd, workspaceRoots());
    return searchProject({ ...opts, cwd });
  });

  // ── Git (review pane) ──────────────────────────────────────────────────
  // Same backend as the git:* IPC, so the web/remote mirror reviews the host's
  // work tree exactly as the desktop does. A failed git command (non-zero exit,
  // not-a-work-tree) rejects the call; the renderer surfaces git's stderr.
  //
  // The review-pane git surface moved out of claudemon into the
  // host (gitService.ts), so its remote-reachable entry point is now these bus
  // capabilities. Every one takes a caller-supplied `cwd`; without confinement a
  // remote/token-holding client could commit or push to — or read the diff of —
  // any git repo the desktop user can write, and a symlinked `cwd` could point
  // outside the intended repo (the finding's original concern). We therefore
  // canonicalize and contain `cwd` to the same workspace roots as fs.* (#8): the
  // live agent cwds the review pane legitimately operates on, plus the config
  // stores.
  // canonicalization resolves symlinks before the check, so a symlinked cwd can't
  // escape the roots. The local desktop IPC path is unchanged: it's the trusted
  // user reviewing their own repos, and this containment only guards the bus.
  // Returns the CANONICAL cwd, which is what gitService is then run in: the
  // directory that was checked and the directory git runs in have to be the same
  // one, or a symlinked cwd is validated in one place and used in another.
  const guardGitCwd = (cap: string, cwd: string): string =>
    assertPathAllowed(cap, cwd, workspaceRoots());
  registerCapability('git.status', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    if (!cwd) throw new Error('git.status requires { cwd }');
    return git.status(guardGitCwd('git.status', cwd));
  });
  registerCapability('git.log', (params: unknown) => {
    const { cwd, limit } = (params ?? {}) as { cwd?: string; limit?: number };
    if (!cwd) throw new Error('git.log requires { cwd }');
    return git.log(guardGitCwd('git.log', cwd), limit).then((commits) => ({ commits }));
  });
  registerCapability('git.diff', async (params: unknown) => {
    const {
      cwd,
      path: filePath,
      staged,
      untracked,
    } = (params ?? {}) as {
      cwd?: string;
      path?: string;
      staged?: boolean;
      untracked?: boolean;
    };
    if (!cwd) throw new Error('git.diff requires { cwd }');
    const canonicalCwd = guardGitCwd('git.diff', cwd);
    // Guarding cwd alone was not enough here. For a tracked diff `path` is a
    // repo pathspec git resolves inside the work tree, but with `untracked` it
    // becomes an operand of `git diff --no-index -- /dev/null <path>`, where git
    // reads it as a plain FILESYSTEM path — so an absolute or ../-laden value
    // rendered any file on the host as an all-added diff, straight past the cwd
    // confinement.
    //
    // The yardstick is the work-tree root, on both counts. gitService runs every
    // command from `rev-parse --show-toplevel` (see its header), so resolving
    // against `cwd` would check a different file than git opens whenever the
    // agent cwd is a subdirectory — the ordinary monorepo case. And the root is
    // also the right *boundary*: the review pane diffs the paths `git.status`
    // printed, which are root-relative and routinely name files in a sibling
    // subtree of the agent cwd, while confining to the repo concedes nothing a
    // path-less `git.diff` (the whole tree's diff) doesn't already hand over.
    let operand = filePath;
    if (filePath) {
      const root = (await git.workRoot(canonicalCwd)) ?? canonicalCwd;
      // Anchor a relative pathspec on the work-tree root by plain concatenation,
      // NOT path.resolve/path.join: those collapse a `link/..` pair textually
      // before any symlink is read, which is precisely the check-path /
      // opened-path split the component walk exists to close. The walk inside
      // assertPathAllowed does the resolving.
      const anchored = path.isAbsolute(filePath)
        ? filePath
        : root.endsWith(path.sep)
          ? root + filePath
          : root + path.sep + filePath;
      const canonicalFile = assertPathAllowed('git.diff', anchored, [root]);
      // git runs from the work-tree root, so it receives the validated path
      // expressed from that root: the operand is a function of the CANONICAL
      // path, never of the caller's string. (Root-relative is what git wants for
      // a tracked pathspec; the absolute form is the fallback for the degenerate
      // "the path IS the root" case.)
      const canonicalRootPath = canonicalRoot(root) ?? root;
      operand = path.relative(canonicalRootPath, canonicalFile) || canonicalFile;
    }
    return { diff: await git.diff(canonicalCwd, operand, staged, untracked) };
  });
  registerCapability('git.numstat', (params: unknown) => {
    const { cwd, staged } = (params ?? {}) as { cwd?: string; staged?: boolean };
    if (!cwd) throw new Error('git.numstat requires { cwd }');
    return git.numstat(guardGitCwd('git.numstat', cwd), staged).then((files) => ({ files }));
  });
  registerCapability('git.commitDiff', (params: unknown) => {
    const { cwd, hash, path } = (params ?? {}) as { cwd?: string; hash?: string; path?: string };
    if (!cwd || !hash) throw new Error('git.commitDiff requires { cwd, hash }');
    return git
      .commitDiff(guardGitCwd('git.commitDiff', cwd), hash, path)
      .then((diff) => ({ diff }));
  });
  registerCapability('git.commitNumstat', (params: unknown) => {
    const { cwd, hash } = (params ?? {}) as { cwd?: string; hash?: string };
    if (!cwd || !hash) throw new Error('git.commitNumstat requires { cwd, hash }');
    return git
      .commitNumstat(guardGitCwd('git.commitNumstat', cwd), hash)
      .then((files) => ({ files }));
  });
  registerCapability('git.stage', (params: unknown) => {
    const { cwd, path } = (params ?? {}) as { cwd?: string; path?: string };
    if (!cwd) throw new Error('git.stage requires { cwd }');
    return git.stage(guardGitCwd('git.stage', cwd), path).then((output) => ({ ok: true, output }));
  });
  registerCapability('git.unstage', (params: unknown) => {
    const { cwd, path } = (params ?? {}) as { cwd?: string; path?: string };
    if (!cwd) throw new Error('git.unstage requires { cwd }');
    return git
      .unstage(guardGitCwd('git.unstage', cwd), path)
      .then((output) => ({ ok: true, output }));
  });
  registerCapability('git.commit', (params: unknown) => {
    const { cwd, message } = (params ?? {}) as { cwd?: string; message?: string };
    if (!cwd || typeof message !== 'string')
      throw new Error('git.commit requires { cwd, message }');
    return git
      .commit(guardGitCwd('git.commit', cwd), message)
      .then((output) => ({ ok: true, output }));
  });
  registerCapability('git.push', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    if (!cwd) throw new Error('git.push requires { cwd }');
    return git.push(guardGitCwd('git.push', cwd)).then((output) => ({ ok: true, output }));
  });
}
