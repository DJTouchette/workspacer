/**
 * Real capabilities the main process exposes on the hub bus. These are the
 * inverse of events — things a plugin (or, later, Claude via the MCP facade)
 * can *ask workspacer to do*. Kept small and explicit; each is a future MCP tool.
 */

import { Notification } from 'electron';
import * as os from 'os';
import * as fs from 'fs';
import * as path from 'path';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { agentHandoffBrief } from './agentHandoff';
import { spawnManagedAgent } from './managedSpawn';
import { spawnClaudeAgent } from './claudeSpawn';
import { resolveAgentBinary, checkAllProviders, type AgentProvider } from './agentProviders';
import { claudeProfiles } from './claudeProfiles';
import { registerCapability } from './hubClient';
import { appIconPath } from '../lib/appIcon';
import { DELEGATE_CATALOG_TO_BRAIN } from './brainDelegation';
import { configService, getConfigDir } from './configService';
import { listClaudeModels } from './claudeModels';
import { libraryService } from './libraryService';
import { sessionService } from './sessionService';
import { sessionHistory } from './sessionHistory';
import { layoutService } from './layoutService';
import { listClaudeSessionsForDir } from './claudeSessionList';
import { readTextFile, writeTextFile, listDir } from './fileService';
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
    try { fs.accessSync(gitBash); return gitBash; } catch {}
    try { require('child_process').execSync('where pwsh.exe', { stdio: 'ignore' }); return 'pwsh.exe'; } catch {}
    return 'powershell.exe';
  }
  return process.env.SHELL || '/bin/sh';
}

// ── Filesystem path confinement for fs.* / search.project (SECURITY.md #8) ──
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
//   - the workspacer config dir — its own settings / library / handoff files
//
// The directory *picker* (fs.listDir) additionally allows browsing the home tree,
// since its whole job is choosing a not-yet-open working directory for a new agent
// (it only lists non-hidden directory names, never file contents). Note this also
// intersects a plugin's own fs grant with these roots; a plugin needing fs access
// to a root outside the workspace would need that root added here (or a per-caller
// identity seam) — acceptable today since plugin fs grants target project files,
// which are agent cwds.

/**
 * Canonicalize `p`: absolute, with `..` and symlinks resolved. For a target that
 * doesn't exist yet (e.g. a file fs.write is about to create) it resolves the
 * longest existing ancestor and re-appends the missing tail, so a write can't be
 * aimed outside a root through a not-yet-created intermediate, and a symlink along
 * the existing prefix is still followed. Throws on any non-ENOENT error mid-walk
 * (permission, etc.) so the caller fails closed.
 */
function canonicalizePath(p: string): string {
  let abs = path.resolve(p);
  let rem = '';
  for (;;) {
    try {
      const real = fs.realpathSync(abs);
      return rem ? path.join(real, rem) : real;
    } catch (err) {
      if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err; // fail closed
      const parent = path.dirname(abs);
      if (parent === abs) return rem ? path.join(abs, rem) : abs; // reached fs root
      rem = rem ? path.join(path.basename(abs), rem) : path.basename(abs);
      abs = parent;
    }
  }
}

/** True when `target` canonicalizes to a location at or inside one of `roots`. */
function pathWithinRoots(roots: string[], target: string): boolean {
  let ct: string;
  try {
    ct = canonicalizePath(target);
  } catch {
    return false; // couldn't verify → deny
  }
  return roots.some((r) => {
    let cr: string;
    try { cr = fs.realpathSync(r); } catch { cr = path.resolve(r); }
    return ct === cr || ct.startsWith(cr + path.sep);
  });
}

/** Workspace roots for content-touching fs.* calls: live agent cwds + config dir. */
function workspaceRoots(): string[] {
  const roots = new Set<string>();
  for (const s of claudeSessionStore.getAllSnapshots()) {
    if (s.cwd) roots.add(s.cwd);
  }
  roots.add(getConfigDir());
  return [...roots];
}

/** Broader roots for the directory picker: the home tree plus the workspace roots. */
function browseRoots(): string[] {
  return [os.homedir(), ...workspaceRoots()];
}

/** Reject a call whose path escapes the allowed roots. */
function assertPathAllowed(cap: string, target: string, roots: string[]): void {
  if (!pathWithinRoots(roots, target)) {
    throw new Error(`${cap}: path is outside the allowed workspace (agent cwds + config dir)`);
  }
}

export function registerHubCapabilities(): void {
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
    const { provider, cwd, profileId, model, effort, permissionMode: reqMode, skipPermissions: reqSkip, resumeSessionId, cols, rows, supervisor, mcpFacade, label, parentSessionId, mcpItemIds } =
      (params ?? {}) as {
        provider?: AgentProvider;
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
      console.warn('[hub] agents.spawn: ignoring permission bypass from a bus client — remote spawns never auto-bypass approvals.');
    }
    const skipPermissions = false;
    const permissionMode = reqMode === 'bypassPermissions' || reqMode === 'yolo' ? undefined : reqMode;
    // Managed (Tier-2) backend — Codex / OpenCode / Pi run through claudemon's
    // adapter, not a Claude PTY. Shares the dispatch with the `claude:spawn` IPC
    // handler so this path can't silently fall back to spawning Claude (it did
    // before — `provider` was ignored here, which is why a Codex agent spawned
    // from the web/remote client came up as Claude).
    if (provider && provider !== 'claude') {
      const sessionId = await spawnManagedAgent({
        provider, cwd, model, effort, skipPermissions, resumeSessionId, supervisor, mcpFacade, label, parentSessionId, cols, rows,
      });
      return { sessionId };
    }
    const sessionId = await spawnClaudeAgent({
      cwd, profileId, model, permissionMode, skipPermissions, resumeSessionId,
      supervisor, mcpFacade, label, parentSessionId, cols, rows, mcpItemIds,
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

  // Surface an OS notification.
  registerCapability('notifications.post', (params: unknown) => {
    const { title, body } = (params ?? {}) as { title?: string; body?: string };
    new Notification({ title: title || 'workspacer', body: body || '', icon: appIconPath() ?? undefined }).show();
    return { ok: true };
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

  // Control: live model/effort switch for managed providers (no restart).
  // Remote counterpart of the `claude:setModel` IPC handler; confirmation
  // flows back through the status line, so no store note is needed.
  registerCapability('claude.setModel', async (params: unknown) => {
    const { sessionId, model, effort } = (params ?? {}) as { sessionId?: string; model?: string; effort?: string };
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
    const { sessionId, option, text, answers } = (params ?? {}) as {
      sessionId?: string;
      option?: number;
      text?: string;
      answers?: string[];
    };
    if (!sessionId) throw new Error('claude.answer requires { sessionId, ... }');
    if (option === undefined && text === undefined && answers === undefined) {
      throw new Error('claude.answer requires one of { option, text, answers }');
    }
    if (option !== undefined) {
      await claudemonSessionClient.input(sessionId, `${option}\r`);
    } else if (text !== undefined) {
      await claudemonSessionClient.input(sessionId, `${text}\r`);
    } else if (answers) {
      for (const a of answers) await claudemonSessionClient.input(sessionId, `${a}\r`);
    }
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
  // context behind a pending approval/question before answering.
  registerCapability('sessions.transcript', async (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('sessions.transcript requires { sessionId }');
    return claudemonSessionClient.getTranscript(sessionId);
  });

  // Read-only: parsed conversation items + latest sequence number. With
  // sinceSeq, returns only items after that sequence — cheap incremental polling
  // so a supervisor digests just the new turns since it last looked.
  registerCapability('sessions.conversation', async (params: unknown) => {
    const { sessionId, sinceSeq } = (params ?? {}) as { sessionId?: string; sinceSeq?: number };
    if (!sessionId) throw new Error('sessions.conversation requires { sessionId }');
    return claudemonSessionClient.getConversation(sessionId, typeof sinceSeq === 'number' ? sinceSeq : undefined);
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
    const { sessionId, cols, rows } = (params ?? {}) as { sessionId?: string; cols?: number; rows?: number };
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
  registerCapability('sessions.snapshots', () => claudeSessionStore.getAllSnapshots());
  registerCapability('sessions.snapshot', (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('sessions.snapshot requires { sessionId }');
    return claudeSessionStore.getSnapshot(sessionId);
  });

  // ── Config (web parity) ────────────────────────────────────────────────
  // Mirror the CONFIG_* IPC handlers so the web renderer loads the real config
  // (theme, keybindings, pane settings) instead of falling back to defaults,
  // and can persist changes from the Settings pane.
  cat('config.get', () => configService.getConfig());
  cat('config.reload', () => configService.reloadConfig());
  cat('config.getPath', () => configService.getConfigPath());
  cat('config.save', (params: unknown) =>
    configService.saveConfig((params ?? {}) as Parameters<typeof configService.saveConfig>[0]));

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
    const { provider, cwd } = (params ?? {}) as { provider?: 'codex' | 'opencode' | 'pi'; cwd?: string };
    if (provider !== 'codex' && provider !== 'opencode' && provider !== 'pi') {
      throw new Error("providers.listModels requires { provider: 'codex'|'opencode'|'pi' }");
    }
    const customBin = configService.getConfig().agents?.binaries?.[provider] ?? '';
    return claudemonSessionClient.listProviderModels(provider, cwd, resolveAgentBinary(provider, customBin));
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
    const { name, configDir, extraArgs, mcpItemIds } = (params ?? {}) as { name?: string; configDir?: string; extraArgs?: string[]; mcpItemIds?: string[] };
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
  cat('library.list', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    return libraryService.list(cwd);
  });
  cat('library.save', (params: unknown) => libraryService.save((params ?? {}) as any));
  cat('library.remove', (params: unknown) => {
    const { scope, id, cwd, kind } = (params ?? {}) as {
      scope?: 'global' | 'project' | 'claude'; id?: string; cwd?: string; kind?: 'prompt' | 'skill' | 'agent';
    };
    if (!scope || !id) throw new Error('library.remove requires { scope, id }');
    libraryService.remove(scope, id, cwd, kind);
    return { ok: true };
  });

  // ── Analytics ──────────────────────────────────────────────────────────
  registerCapability('analytics.summary', (params: unknown) => {
    const { provider } = (params ?? {}) as { provider?: string };
    return sessionHistory.summary(provider);
  });
  registerCapability('analytics.recent', (params: unknown) => {
    const { limit, provider } = (params ?? {}) as { limit?: number; provider?: string };
    return sessionHistory.recent(limit, provider);
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
    const resolved = path.resolve(p && p.trim() ? p.replace(/^~/, home) : home);
    // Browsing is limited to the home tree + live agent cwds so a remote client
    // can pick a project dir but can't enumerate /etc, /root, or other users' homes.
    assertPathAllowed('fs.listDir', resolved, browseRoots());
    let dirs: string[] = [];
    try {
      dirs = fs.readdirSync(resolved, { withFileTypes: true })
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
    assertPathAllowed('fs.read', p, workspaceRoots());
    return readTextFile(p);
  });
  cat('fs.write', (params: unknown) => {
    const { path: p, contents } = (params ?? {}) as { path?: string; contents?: string };
    if (!p) throw new Error('fs.write requires a path');
    assertPathAllowed('fs.write', p, workspaceRoots());
    return writeTextFile(p, contents ?? '');
  });
  // Files-included, gitignore-aware listing for the editor's file tree (web client).
  cat('fs.listEntries', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.listEntries requires a path');
    assertPathAllowed('fs.listEntries', p, workspaceRoots());
    return listDir(p);
  });

  // ── File watch (editor external-change detection, web client) ──────────
  // Starts/stops a host-side watch; the watcher's global emit sink (installed in
  // ipc.ts) mirrors every change onto the bus as a `fs.changed` event carrying
  // { path, eventType }, which webBackend subscribes to and filters by path.
  registerCapability('fs.watch', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.watch requires a path');
    assertPathAllowed('fs.watch', p, workspaceRoots());
    startWatch(p);
    return { ok: true };
  });
  registerCapability('fs.unwatch', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.unwatch requires a path');
    assertPathAllowed('fs.unwatch', p, workspaceRoots());
    stopWatch(p);
    return { ok: true };
  });

  // ── Project search (editor search sidebar, web client) ─────────────────
  // Same ripgrep backend as the search:project IPC.
  registerCapability('search.project', (params: unknown) => {
    const opts = (params ?? {}) as Parameters<typeof searchProject>[0];
    if (!opts.query) throw new Error('search.project requires { query, cwd }');
    if (!opts.cwd) throw new Error('search.project requires { query, cwd }');
    assertPathAllowed('search.project', opts.cwd, workspaceRoots());
    return searchProject(opts);
  });

  // ── Git (review pane) ──────────────────────────────────────────────────
  // Same backend as the git:* IPC, so the web/remote mirror reviews the host's
  // work tree exactly as the desktop does. A failed git command (non-zero exit,
  // not-a-work-tree) rejects the call; the renderer surfaces git's stderr.
  //
  // SECURITY.md #6: the review-pane git surface moved out of claudemon into the
  // host (gitService.ts), so its remote-reachable entry point is now these bus
  // capabilities. Every one takes a caller-supplied `cwd`; without confinement a
  // remote/token-holding client could commit or push to — or read the diff of —
  // any git repo the desktop user can write, and a symlinked `cwd` could point
  // outside the intended repo (the finding's original concern). We therefore
  // canonicalize and contain `cwd` to the same workspace roots as fs.* (#8): the
  // live agent cwds the review pane legitimately operates on, plus the config dir.
  // canonicalization resolves symlinks before the check, so a symlinked cwd can't
  // escape the roots. The local desktop IPC path is unchanged: it's the trusted
  // user reviewing their own repos, and this containment only guards the bus.
  const guardGitCwd = (cap: string, cwd: string): void =>
    assertPathAllowed(cap, cwd, workspaceRoots());
  registerCapability('git.status', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    if (!cwd) throw new Error('git.status requires { cwd }');
    guardGitCwd('git.status', cwd);
    return git.status(cwd);
  });
  registerCapability('git.diff', (params: unknown) => {
    const { cwd, path, staged, untracked } = (params ?? {}) as {
      cwd?: string; path?: string; staged?: boolean; untracked?: boolean;
    };
    if (!cwd) throw new Error('git.diff requires { cwd }');
    guardGitCwd('git.diff', cwd);
    return git.diff(cwd, path, staged, untracked).then((diff) => ({ diff }));
  });
  registerCapability('git.numstat', (params: unknown) => {
    const { cwd, staged } = (params ?? {}) as { cwd?: string; staged?: boolean };
    if (!cwd) throw new Error('git.numstat requires { cwd }');
    guardGitCwd('git.numstat', cwd);
    return git.numstat(cwd, staged).then((files) => ({ files }));
  });
  registerCapability('git.stage', (params: unknown) => {
    const { cwd, path } = (params ?? {}) as { cwd?: string; path?: string };
    if (!cwd) throw new Error('git.stage requires { cwd }');
    guardGitCwd('git.stage', cwd);
    return git.stage(cwd, path).then((output) => ({ ok: true, output }));
  });
  registerCapability('git.unstage', (params: unknown) => {
    const { cwd, path } = (params ?? {}) as { cwd?: string; path?: string };
    if (!cwd) throw new Error('git.unstage requires { cwd }');
    guardGitCwd('git.unstage', cwd);
    return git.unstage(cwd, path).then((output) => ({ ok: true, output }));
  });
  registerCapability('git.commit', (params: unknown) => {
    const { cwd, message } = (params ?? {}) as { cwd?: string; message?: string };
    if (!cwd || typeof message !== 'string') throw new Error('git.commit requires { cwd, message }');
    guardGitCwd('git.commit', cwd);
    return git.commit(cwd, message).then((output) => ({ ok: true, output }));
  });
  registerCapability('git.push', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    if (!cwd) throw new Error('git.push requires { cwd }');
    guardGitCwd('git.push', cwd);
    return git.push(cwd).then((output) => ({ ok: true, output }));
  });
}
