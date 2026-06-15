/**
 * Real capabilities the main process exposes on the hub bus. These are the
 * inverse of events — things a plugin (or, later, Claude via the MCP facade)
 * can *ask workspacer to do*. Kept small and explicit; each is a future MCP tool.
 */

import { Notification } from 'electron';
import * as os from 'os';
import * as fs from 'fs';
import * as path from 'path';
import { randomUUID } from 'crypto';
import { claudeSessionStore } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { buildClaudeArgv } from './claudeResolver';
import { claudeProfiles } from './claudeProfiles';
import { registerCapability } from './hubClient';
import { configService } from './configService';
import { listClaudeModels } from './claudeModels';
import { libraryService } from './libraryService';
import { sessionService } from './sessionService';
import { sessionHistory } from './sessionHistory';
import { layoutService } from './layoutService';
import { listClaudeSessionsForDir } from './claudeSessionList';
import { readTextFile, writeTextFile, listDir } from './fileService';
import { startWatch, stopWatch } from './fileWatchService';
import { searchProject } from './searchService';
import * as terminalShare from './terminalShare';
import { IPC } from '../shared/ipcChannels';
import type { SessionData, LayoutInput, ProfileUpdate } from '../shared/ipcTypes';
import { supervisorMcpConfigPath, SUPERVISOR_SYSTEM_PROMPT } from './mcpConfig';

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

export function registerHubCapabilities(): void {
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

  // Control: send a prompt to an agent. Prefers claudemon's mode-gated /message
  // (it appends the carriage return for us). When the session isn't at an input
  // prompt — e.g. the agent is mid-turn — /message 409s; rather than silently
  // dropping the text (which made the remote "break" after the first message),
  // mirror the desktop ClaudePane fallback and type straight into the PTY so
  // follow-up messages queue into claude's input like any other keystrokes.
  registerCapability('agents.sendMessage', async (params: unknown) => {
    const { sessionId, text } = (params ?? {}) as { sessionId?: string; text?: string };
    if (!sessionId || typeof text !== 'string') {
      throw new Error('agents.sendMessage requires { sessionId, text }');
    }
    const res = await claudemonSessionClient.message(sessionId, text);
    if (!res.ok) {
      await claudemonSessionClient.input(sessionId, text);
      await new Promise((r) => setTimeout(r, 50));
      await claudemonSessionClient.input(sessionId, '\r');
    }
    return { ok: true };
  });

  // Control: spawn a brand-new Claude Code agent session. The hub/MCP
  // counterpart of the `claude:spawn` IPC handler — lets a remote client (or
  // Claude via the MCP facade) start a fresh agent in a directory. Mirrors the
  // IPC path exactly (pinned session id, profile env, argv), then returns the
  // new sessionId so the caller can immediately drive it with the other
  // capabilities. The session runs headless in claudemon; a desktop pane can
  // attach to it later via the normal attach flow.
  registerCapability('agents.spawn', async (params: unknown) => {
    const { cwd, profileId, model, skipPermissions, resumeSessionId, cols, rows, supervisor, label, parentSessionId } =
      (params ?? {}) as {
        cwd?: string;
        profileId?: string;
        model?: string;
        skipPermissions?: boolean;
        resumeSessionId?: string;
        cols?: number;
        rows?: number;
        supervisor?: boolean;
        label?: string;
        parentSessionId?: string;
      };
    const profile = profileId ? claudeProfiles.getProfile(profileId) : undefined;
    const env: Record<string, string> = {};
    if (profile?.configDir) {
      env.CLAUDE_CONFIG_DIR = profile.configDir.replace(/^~/, os.homedir());
    }
    // Pin the id so claude's transcript filename matches our id (see ipc.ts).
    const sessionId = resumeSessionId || randomUUID();
    // Record name/parent before the session registers so adopted cards are
    // enriched from the very first hook event.
    claudeSessionStore.setSpawnMeta(sessionId, { label, parentSessionId });
    const argv = buildClaudeArgv({
      extraArgs: profile?.extraArgs,
      resumeSessionId,
      model,
      skipPermissions,
      sessionId,
      // Supervisor sessions get the MCP facade config + pre-allowed tools +
      // role prompt injected so the agent can observe and drive the fleet.
      // Also tell the supervisor its own session id so it can pass parentSessionId
      // when spawning workers, making them appear nested in the UI.
      ...(supervisor && {
        mcpConfig: supervisorMcpConfigPath(),
        appendSystemPrompt: `${SUPERVISOR_SYSTEM_PROMPT}\n\nYour own workspacer session id is ${sessionId}. When you spawn worker agents with spawn_agent, pass parentSessionId:"${sessionId}" and a short label so they appear nested under you in the UI.`,
        allowedTools: ['mcp__workspacer'],
      }),
    });
    const resolvedCwd = cwd ?? process.env.HOME ?? os.homedir();
    const id = await claudemonSessionClient.spawn({ argv, cwd: resolvedCwd, cols, rows, env, sessionId });
    return { sessionId: id };
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
    new Notification({ title: title || 'workspacer', body: body || '' }).show();
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
  registerCapability('config.get', () => configService.getConfig());
  registerCapability('config.reload', () => configService.reloadConfig());
  registerCapability('config.getPath', () => configService.getConfigPath());
  registerCapability('config.save', (params: unknown) =>
    configService.saveConfig((params ?? {}) as Parameters<typeof configService.saveConfig>[0]));

  // ── Model picker (web parity) ──────────────────────────────────────────
  registerCapability('claude.listModels', () => listClaudeModels());

  // ── Saved sessions (workspace layouts) ─────────────────────────────────
  // Mirror the SESSION_* IPC handlers so the web client can list/load/save the
  // saved agent arrangements (the session picker).
  registerCapability('sessions.list', () => sessionService.listSessions());
  registerCapability('sessions.load', (params: unknown) => {
    const { filename } = (params ?? {}) as { filename?: string };
    if (!filename) throw new Error('sessions.load requires { filename }');
    return sessionService.loadSession(filename);
  });
  registerCapability('sessions.save', (params: unknown) => {
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
  registerCapability('sessions.delete', (params: unknown) => {
    const { filename } = (params ?? {}) as { filename?: string };
    if (!filename) throw new Error('sessions.delete requires { filename }');
    sessionService.deleteSession(filename);
    return { ok: true };
  });

  // ── Layout templates ───────────────────────────────────────────────────
  registerCapability('layouts.list', () => layoutService.list());
  registerCapability('layouts.save', (params: unknown) => layoutService.save((params ?? {}) as LayoutInput));
  registerCapability('layouts.delete', (params: unknown) => {
    const { id } = (params ?? {}) as { id?: string };
    if (!id) throw new Error('layouts.delete requires { id }');
    layoutService.remove(id);
    return { ok: true };
  });

  // ── Claude profiles ────────────────────────────────────────────────────
  registerCapability('claude.profiles.list', () => claudeProfiles.getProfiles());
  registerCapability('claude.profiles.add', (params: unknown) => {
    const { name, configDir, extraArgs } = (params ?? {}) as { name?: string; configDir?: string; extraArgs?: string[] };
    if (!name) throw new Error('claude.profiles.add requires { name }');
    return claudeProfiles.addProfile(name, configDir ?? '', extraArgs ?? []);
  });
  registerCapability('claude.profiles.update', (params: unknown) => {
    const { id, updates } = (params ?? {}) as { id?: string; updates?: ProfileUpdate };
    if (!id) throw new Error('claude.profiles.update requires { id, updates }');
    return claudeProfiles.updateProfile(id, updates ?? ({} as ProfileUpdate));
  });
  registerCapability('claude.profiles.remove', (params: unknown) => {
    const { id } = (params ?? {}) as { id?: string };
    if (!id) throw new Error('claude.profiles.remove requires { id }');
    claudeProfiles.removeProfile(id);
    return { ok: true };
  });

  // ── Claude session discovery (resume picker) ───────────────────────────
  registerCapability('claude.sessionsForDir', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    if (!cwd) throw new Error('claude.sessionsForDir requires { cwd }');
    return listClaudeSessionsForDir(cwd);
  });

  // ── Library (reusable prompts + skills) ────────────────────────────────
  registerCapability('library.list', (params: unknown) => {
    const { cwd } = (params ?? {}) as { cwd?: string };
    return libraryService.list(cwd);
  });
  registerCapability('library.save', (params: unknown) => libraryService.save((params ?? {}) as any));
  registerCapability('library.remove', (params: unknown) => {
    const { scope, id, cwd, kind } = (params ?? {}) as {
      scope?: 'global' | 'project' | 'claude'; id?: string; cwd?: string; kind?: 'prompt' | 'skill' | 'agent';
    };
    if (!scope || !id) throw new Error('library.remove requires { scope, id }');
    libraryService.remove(scope, id, cwd, kind);
    return { ok: true };
  });

  // ── Analytics ──────────────────────────────────────────────────────────
  registerCapability('analytics.summary', () => sessionHistory.summary());
  registerCapability('analytics.recent', (params: unknown) => {
    const { limit } = (params ?? {}) as { limit?: number };
    return sessionHistory.recent(limit);
  });

  // ── Approval gate + host cwd ───────────────────────────────────────────
  registerCapability('claude.gate', (params: unknown) => {
    const { sessionId, on } = (params ?? {}) as { sessionId?: string; on?: boolean };
    if (!sessionId) throw new Error('claude.gate requires { sessionId, on }');
    return claudemonSessionClient.setGate(sessionId, !!on);
  });
  registerCapability('app.getCwd', () => process.cwd());

  // ── Host filesystem browsing (web folder picker) ───────────────────────
  // The web client can't open a native OS dialog, so it browses the host's
  // directories through this to choose a working directory for a new agent.
  // Directories only (you spawn an agent *in* a folder); hidden entries skipped.
  registerCapability('fs.listDir', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    const home = os.homedir();
    const resolved = path.resolve(p && p.trim() ? p.replace(/^~/, home) : home);
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
  registerCapability('fs.read', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.read requires a path');
    return readTextFile(p);
  });
  registerCapability('fs.write', (params: unknown) => {
    const { path: p, contents } = (params ?? {}) as { path?: string; contents?: string };
    if (!p) throw new Error('fs.write requires a path');
    return writeTextFile(p, contents ?? '');
  });
  // Files-included, gitignore-aware listing for the editor's file tree (web client).
  registerCapability('fs.listEntries', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.listEntries requires a path');
    return listDir(p);
  });

  // ── File watch (editor external-change detection, web client) ──────────
  // Starts/stops a host-side watch; the watcher's global emit sink (installed in
  // ipc.ts) mirrors every change onto the bus as a `fs.changed` event carrying
  // { path, eventType }, which webBackend subscribes to and filters by path.
  registerCapability('fs.watch', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.watch requires a path');
    startWatch(p);
    return { ok: true };
  });
  registerCapability('fs.unwatch', (params: unknown) => {
    const { path: p } = (params ?? {}) as { path?: string };
    if (!p) throw new Error('fs.unwatch requires a path');
    stopWatch(p);
    return { ok: true };
  });

  // ── Project search (editor search sidebar, web client) ─────────────────
  // Same ripgrep backend as the search:project IPC.
  registerCapability('search.project', (params: unknown) => {
    const opts = (params ?? {}) as Parameters<typeof searchProject>[0];
    if (!opts.query) throw new Error('search.project requires { query, cwd }');
    return searchProject(opts);
  });
}
