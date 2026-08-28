/**
 * Real capabilities the main process exposes on the hub bus. These are the
 * inverse of events — things a plugin (or, later, Claude via the MCP facade)
 * can *ask workspacer to do*. Kept small and explicit; each is a future MCP tool.
 */

import { Notification, shell } from 'electron';
import { randomUUID } from 'crypto';
import * as os from 'os';
import * as fs from 'fs';
import * as path from 'path';
import { claudeSessionStore, contextTokensFromStatusLine } from './claudeSessionStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { applyLiveEffort } from './liveEffort';
import { agentHandoffBrief } from './agentHandoff';
import { spawnManagedAgent } from './managedSpawn';
import { spawnClaudeAgent } from './claudeSpawn';
import { resolveAgentBinary, checkAllProviders, type AgentProvider } from './agentProviders';
import { byteCompare } from '../lib/providerParity';
import { resolveTerminalShell } from '../lib/shellAllowlist';
import { normalizeSpawnCwd } from '../lib/spawnCwd';
import { createWorktree } from './worktreeService';
import {
  assertNoPermissionBypass,
  isPermissionEscalation,
  permissionModeMeansBypass,
} from '../lib/permissionBypass';
import type { RemoteTokenScope } from '../shared/ipcTypes';
import { claudeProfiles, scrubBypassProfile } from './claudeProfiles';
import { registerCapability, callHub, emitToRenderer } from './hubClient';
import { agentNotifier } from './agentNotifier';
import { appIconPath } from '../lib/appIcon';
import { dropHostTrusted } from '../lib/hostTrustedConfig';
import {
  assertPathAllowed,
  canonicalRoot,
  configStoreRoots,
  containsCanonical,
} from '../lib/pathConfinement';
import { snapshotGrantsFsRoot } from '../lib/snapshotLiveness';
import { DELEGATE_CATALOG_TO_BRAIN } from './brainDelegation';
import { configService, getConfigDir } from './configService';
import { listClaudeModels } from './claudeModels';
import { libraryService, type ClaudeOrigin, type LibraryFileGuard } from './libraryService';
// Not from libraryService: every suite that mocks that module away still needs
// the kind vocabulary library.list validates its filter against.
import { LIBRARY_KINDS, type LibraryKind } from '../shared/libraryKinds';
import { renderDispatchTemplate } from '../lib/dispatchTemplate';
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
import { compactClaudeSnapshotForBackground } from '../shared/compactClaudeSnapshot';
import { ensureSupervisorHome } from './supervisorSkill';
import { scrubBootDocumentAgents } from '../lib/bootDocumentScrub';
import { isAsciiBlank } from '../lib/asciiWhitespace';
import { appendBriefLine, briefPathFor, parseBriefSection } from './briefService';
import { composeResultLine, hasResultParams } from '../lib/briefResultLine';
import { checkNowSection, liveSessionIds } from './briefCheck';
import { archiveOldestEntries } from './briefBoardService';
import { thresholdWatcher } from './thresholdWatcher';
import { progressReporter } from './progressReporter';

/**
 * How much of a template's rendered first message rides back in the spawn
 * result. A dispatch template is prose a human wrote, so this is far above any
 * real one — it exists so a pathological template (a generated wall of text, a
 * placeholder filled with a file dump) cannot bloat a tool result that a
 * manager reads inside its own context window. Past the cap the field is
 * truncated and `renderedMessageTruncated: true` says so, because a silently
 * clipped echo is worse than no echo: the caller would verify a render it never
 * actually saw the end of.
 */
const RENDERED_MESSAGE_CAP = 16_000;

/**
 * The `agents.spawn` answer. `messageQueued` is this host's ACKNOWLEDGEMENT
 * that it took delivery of the first prompt — claudemon queues it inside the
 * spawn handler, and an unconfirmed queue raises a banner rather than being
 * swallowed (claudemonSessionClient.deliverFirstMessage). A dispatcher that
 * does not see it true (an older federated peer, a lagging headless brain)
 * knows to send the prompt itself instead of assuming it landed.
 *
 * Omitted entirely when no message was asked for, so the result shape stays
 * byte-for-byte what every other spawn has always answered. TWIN: the brain's
 * `spawnResult` (cmd/brain/handlers.go).
 */
function spawnResult(
  sessionId: string,
  message: string | undefined,
  escalation: { fullAccess: boolean; scrubbed: string[] },
  /** The message was RENDERED FROM A TEMPLATE, so the caller has not seen it.
   *  A plain `message` spawn gets no echo — the caller wrote the text. */
  renderedFromTemplate = false,
): Record<string, unknown> {
  // NO SILENT DOWNGRADES (2026-08-26). `fullAccess` is what the session ACTUALLY
  // runs with — not what was requested — and rides EVERY spawn answer, so a
  // client whose user clicked "full access" can see it did not happen without
  // knowing which fields exist. `escalationScrubbed` then names what was taken
  // away (by the hub router or by the clamps below); omitted when nothing was,
  // so an ordinary spawn keeps exactly today's shape.
  // TWIN: cmd/brain/handlers.go spawnResult.
  const out: Record<string, unknown> = { sessionId, fullAccess: escalation.fullAccess };
  if (escalation.scrubbed.length) out.escalationScrubbed = escalation.scrubbed;
  // THE RENDER, echoed back — the point being that a template spawn is the one
  // case where the dispatcher does not know what it sent. Before this, checking
  // that {{task}} landed where it should meant agents.getConversation, which has
  // no small-slice option and returns the whole transcript.
  if (renderedFromTemplate && typeof message === 'string' && message !== '') {
    out.renderedMessage = message.slice(0, RENDERED_MESSAGE_CAP);
    if (message.length > RENDERED_MESSAGE_CAP) out.renderedMessageTruncated = true;
  }
  if (!sessionId || !message?.trim()) return out;
  // Not "we passed it on" — whether it actually got there. The helper already
  // fell back to a plain send and banners on total failure; reporting true
  // regardless would leave the DISPATCHER (a manager, a peer) believing it
  // dispatched a task, which is the one thing this field exists to prevent.
  const failed = claudemonSessionClient.takeUndeliveredFirstMessage(sessionId);
  out.messageQueued = !failed;
  return out;
}

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

/** Workspace roots for content-touching fs.* calls: live agent cwds + config stores.
 *
 *  LIVE is load-bearing and used to be a word in this comment only: the loop
 *  below added every snapshot's cwd with no state test whatsoever, while the
 *  brain's `agentCwds()` has filtered on `snapshotLive` since it was written. So
 *  one session row granted a root on one provider and was refused by the other,
 *  permanently — and this store's only removal path is a 30-second timer armed
 *  by a SessionEnd hook, so a PTY killed without one (SIGKILL, crash, OOM) kept
 *  `status: 'active'` and kept its directory in the allow-list for the life of
 *  the app process. git.diff / fs.readImage / fs.watch / fs.unwatch are answered
 *  here even under the default catalog delegation, so that was the shipping
 *  configuration. See lib/snapshotLiveness.ts for the clause-by-clause rule. */
function workspaceRoots(): string[] {
  const roots = new Set<string>();
  for (const s of claudeSessionStore.getAllSnapshots()) {
    if (s.cwd && snapshotGrantsFsRoot(s)) roots.add(s.cwd);
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
  // a `workspacer serve` hub, its FULL-scope brain already provides most of
  // this surface. We still register everything below UNCHANGED, on purpose:
  // the hub router is first-registration-wins (services/hub internal/bus/
  // rpc.go), so brain-owned methods are simply withheld from us — partial
  // registration is native to the bus, no pre-negotiation needed.
  //
  // For most of the overlap that really is harmless: both sides proxy the SAME
  // claudemon, so whichever owns agents.*/sessions.*/claude.* answers
  // identically. THREE METHODS ARE NOT, and in the adopted configuration the
  // user loses them:
  //
  //   ADOPTED-DEGRADED: notifications.post — the brain only LOGS it; no OS
  //     toast is raised.
  //   ADOPTED-DEGRADED: analytics.summary — the brain answers an all-zero stub
  //     carrying `unavailable: "headless"`. It is NOT true that the brain
  //     "doesn't provide analytics headlessly": it does, it wins the race, and
  //     main's real SQLite session-history store is never asked.
  //   ADOPTED-DEGRADED: analytics.recent — same stub, an empty row list.
  //   ADOPTED-DEGRADED: claude.setPermissionMode — the switch itself is
  //     identical (both POST claudemon's /permission-mode), but the brain
  //     cannot call claudeSessionStore.notePermissionMode, so THIS process's
  //     `livePermissionMode` is not updated eagerly. The mode pill follows the
  //     daemon's own telemetry instead of flipping on the confirmed reply.
  //   ADOPTED-DEGRADED: claude.setModel — same shape: the requested model is
  //     not noted eagerly (noteRequestedModel), so the context-window figure
  //     does not follow an `opus[1m]` switch until the status line confirms it.
  //   ADOPTED-DEGRADED: claude.setEffort — same again for noteEffort, and this
  //     one is the most visible: for a CLAUDE session that note is the pill's
  //     ONLY truth (effective effort appears in no hook, status line or init
  //     frame), so an adopted hub's effort pill does not move at all.
  //
  // And the agent-facing fleet verbs, which the brain now provides too
  // (cmd/brain/agentops.go, brief.go, readimage.go, recent.go, visibleterm.go).
  // ONE REASON COVERS MOST OF THEM: each implementation below reaches into
  // claudeSessionStore, which is authoritative HERE and does not exist in the
  // brain — it answers from its own projection of claudemon plus the spawn
  // metadata it recorded itself. So in the adopted configuration these keep
  // working and see a narrower world.
  //
  //   ADOPTED-DEGRADED: agents.reportProgress — the brain routes to the parent
  //     in ITS spawn metadata, so a worker THIS process spawned is refused with
  //     "not a tracked session" instead of reaching its manager.
  //   ADOPTED-DEGRADED: agents.notifyWhen — armed against the brain's snapshot
  //     projection; a session it has no row for is refused at arm time.
  //   ADOPTED-DEGRADED: agents.close — "forgotten" means removed from the
  //     BRAIN's store. This process's own row, and therefore the sidebar card,
  //     is untouched.
  //   ADOPTED-DEGRADED: agents.orphans — it can only report parents it recorded
  //     itself, so a manager THIS process spawned and that then died is not
  //     listed as a candidate at all.
  //   ADOPTED-DEGRADED: agents.reparent — moves the link in the brain's spawn
  //     metadata only; claudeSessionStore is not updated, so a wake raised HERE
  //     still goes to the retired manager.
  //   ADOPTED-DEGRADED: terminals.open — the brain has no renderer to emit to
  //     and publishes a `facade.openTerminal` bus event instead, which this app
  //     does not subscribe to. An agent's open_terminal reaches a WEB client,
  //     not this window.
  //   ADOPTED-DEGRADED: fs.readImage — same guard and same extension
  //     allowlist, but the brain has no image decoder: it inlines the original
  //     bytes (the fallback branch imagePreview already has) and refuses an
  //     image too large to inline, where this side would have downscaled it.
  //   ADOPTED-DEGRADED: sessions.recent — same daemon rows in the same order,
  //     but the SQLite session-history join is unavailable to the brain, so
  //     `model` and `costUSD` come back empty and `title` is never
  //     transcript-derived.
  //   ADOPTED-DEGRADED: brief.check — the READING is identical (the brain ports
  //     this file's document model, held to it case by case by
  //     contracts/brief-board-cases.json), but the LIVENESS source is not:
  //     this side matches a Now line's session against claudeSessionStore,
  //     which holds federated peer rows and rows this app spawned itself, and
  //     the brain matches against its own projection of claudemon. Under an
  //     adopted hub a Now line naming a session only THIS process knows about
  //     is reported stale. The report never edits anything, so the cost is a
  //     false flag a manager reads and dismisses, not a lost line.
  //
  // brief.append and brief.archive are the ones that are NOT degraded: the
  // brain's ports are the same additive insert and the same whole-line splice,
  // under the same lock and compare-and-swap, writing the same files through
  // the same path guard.
  //
  // The full classification (every overlapping method, equivalent vs degraded,
  // with the reason) is enumerated and enforced by
  // services/hub/cmd/brain/delegation_guard_test.go
  // TestMainOwnedCapabilitiesDoNotCollideWithTheBrain, which fails if a new
  // overlap appears undeclared — or if a degraded one stops being named here.
  // hubClient warns with the withheld set from the `registered` ack.

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
      // Who dispatched this session, when anything did — the fleet's wake
      // routing key (agents.reparent / reparentChildren re-points exactly this
      // field). It is here for the ONE case the handoff file cannot cover: a
      // manager that CRASHED wrote no handoff, so its successor has no
      // predecessor id to adopt from. With this on the row the successor can
      // derive the candidates instead — group the rows by parentSessionId, and
      // any parent id that has no row of its own is a DEAD parent whose
      // children are still running, which is exactly the `fromSessionId`
      // agents.reparent wants. It narrows the manual reconciliation to
      // "which of these dangling parents was mine"; it does not answer that
      // question on its own (a dead session is evicted from the store, so
      // nothing here says the parent was a manager, or which one was yours).
      //
      // NOT a widening of the view tier. `sessions.snapshots` sits in the same
      // viewMethods allowlist and already serves the FULL snapshot — this field
      // included — and the brain's own agents.list has emitted it since
      // enrichSnapshot, because /m nests the fleet on it. This reduced row was
      // the odd one out: a view caller could already read every parent id here
      // by paying for the heavier call.
      parentSessionId: s.parentSessionId ?? null,
      // The two fields that turn a parent id into a NAME. Same convergence
      // argument as parentSessionId above and the same evidence: the brain's
      // agents.list has served both since enrichSnapshot (pinned by
      // cmd/brain/parity_test.go, because /m titles and nests the fleet on
      // them), and the desktop's own `sessions.snapshots` — the other method in
      // the same viewMethods allowlist — already ships the whole snapshot,
      // these included, through compactClaudeSnapshotForBackground's spread. So
      // this is the reduced row catching up with what the view tier could
      // already read by paying for the heavier call; it widens nothing.
      //
      // They are also exactly what succession needs: `label` is the static task
      // title a manager is recognised by, and `isSupervisor` is the difference
      // between "a dispatch of mine" and "another manager's". agents.orphans
      // below answers the same two questions about the DEAD.
      label: s.label ?? null,
      isSupervisor: s.isSupervisor === true,
      // Managed providers (codex/opencode/pi) never populate `s.usage` — their
      // numbers live only on `statusLine`. Fall back to it the same way
      // analyticsWriter.ts does, or every non-Claude row reports all-zero.
      model: s.usage?.model ?? s.statusLine?.modelDisplay ?? null,
      contextTokens: s.usage?.contextTokens ?? contextTokensFromStatusLine(s.statusLine) ?? 0,
      // THE WINDOW: statusLine FIRST. This was the other way round, and every
      // bus client — /m, /app, remote.html, wks-tui, every federated peer —
      // inherited the ordering. `statusLine.contextWindowSize` is what the
      // PROVIDER said about this session (Claude's own statusLine payload,
      // Codex's `model_context_window`); `usage.contextLimit` is what the
      // desktop's own engine worked out from a model id. Preferring the
      // computed value over the reported one is backwards on its face, and it
      // is how two clients came to show different windows for the same session
      // at the same instant.
      //
      // The pair is deliberately NOT taken from one source: a direct token
      // count from the transcript beside a provider-reported window is the most
      // accurate reading available. The derived alternative
      // (`contextTokensFromStatusLine`, which is pct × window) inherits the
      // window's error into the TOKEN count, which is one way an absurd token
      // figure reaches a client.
      //
      // `0` rather than an omitted key: every consumer of this row guards
      // truthily (`u.contextLimit ? … : …`) and reads 0 as unknown, which is
      // what a null limit now means here. What matters is that it is never a
      // guessed 200_000.
      contextLimit: s.statusLine?.contextWindowSize ?? s.usage?.contextLimit ?? 0,
      costUSD: s.usage?.costUSD ?? s.statusLine?.costUSD ?? 0,
      // What the agent is blocked on, if anything — lets a remote client show
      // the actual approval/question instead of a generic "waiting" badge.
      pendingApproval: s.pendingApproval
        ? { toolName: s.pendingApproval.toolName, toolInput: s.pendingApproval.toolInput }
        : null,
      pendingQuestions: s.pendingQuestions ?? null,
      // Epoch ms of the last REAL conversation delta or ambient transition —
      // deliberately not bumped by statusLine ticks (see claudeSessionStore's
      // note at the statusLine handler). That makes it the one field on this
      // row that can catch a wedged agent: `state === 'streaming' && now -
      // lastActivity > 5min` is a session claiming to work with nothing
      // arriving. `state` alone cannot — a wedged session reports `streaming`
      // forever, which is also why an idleSeconds watch never fires on one.
      lastActivity: s.lastActivity,
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
  //
  // THIS is the path the Fleet Manager actually dispatches every worker
  // through (MCP facade → agents.spawn), never the IPC one — so it is the
  // one that matters most for the manager/fleetFullAccess drop
  // managedSpawnOptions.ts documents: all three branches below used to
  // hand-copy their spawn-options object and silently omit both fields,
  // meaning NO Fleet Manager spawned over the bus (codex, opencode, pi, or
  // even Claude) ever came up as isSupervisor, so it never received a single
  // worker-finished wake.
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
      manager,
      fleetFullAccess,
      mcpFacade,
      toolScope,
      pluginTools,
      label,
      parentSessionId,
      mcpItemIds,
      profileGranted,
      yoloGranted,
      escalationScrubbed: hubScrubbed,
      worktree,
      resultSchema: reqResultSchema,
      message: reqMessage,
      template,
      templateParams,
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
      /** Fleet Manager: nudge-eligible parent without the /supervise loop —
       *  see managedSpawnOptions.ts for why dropping this is the load-bearing
       *  bug class this capability must not repeat. */
      manager?: boolean;
      /** Manager only: full-access dispatch grant (config agents.fleetFullAccess);
       *  kept on the wire for record fidelity (fullAccessGrants.ts resolves the
       *  actual grant from config, not this flag). */
      fleetFullAccess?: boolean;
      mcpFacade?: boolean;
      /** Facade tool tier: 'view' | 'triage' | 'operator' (implies the facade).
       *  Not an escalation door: only trusted/operator callers reach
       *  agents.spawn at all, and the tier only ever NARROWS the facade the
       *  legacy mcpFacade flag already granted wholesale. */
      toolScope?: RemoteTokenScope;
      /** Plugin ids whose contributed facade tools the session may use. */
      pluginTools?: string[];
      label?: string;
      parentSessionId?: string;
      mcpItemIds?: string[];
      /** HUB-STAMPED, never caller-supplied: the hub's sanitizeSpawnParams
       *  deletes any incoming copy and re-stamps true only after verifying the
       *  calling token's profilesAllowed grant names this exact profileId (or
       *  the caller is the trusted host). Softens the profile scrub to
       *  scrubRemoteGrantedProfile — configDir kept, bypass args and
       *  mcpItemIds still stripped. TWIN: rpc.go sanitizeSpawnParams. */
      profileGranted?: boolean;
      /** HUB-STAMPED, never caller-supplied (same guarantee as profileGranted):
       *  the calling token's YoloAllowed grant is verified before this is set.
       *  When true, the spawn's requested skipPermissions / bypass mode is
       *  HONORED instead of clamped — the fleet-manager full-access path.
       *  TWIN: rpc.go sanitizeSpawnParams. */
      yoloGranted?: boolean;
      /** HUB-STAMPED, never caller-supplied (same guarantee as the two above —
       *  sanitizeSpawnParams deletes any incoming copy): the spawn-escalation
       *  fields the ROUTER already removed before this call arrived, today just
       *  `profileId` when the calling token may not name that account. Folded
       *  together with this handler's OWN clamps into the result's
       *  `escalationScrubbed`, so no downgrade is silent. */
      escalationScrubbed?: string[];
      /** Run the new agent in a fresh isolated git worktree of `cwd` (its own
       *  branch) instead of the checkout — the fleet manager's ship-task
       *  isolation so parallel work on one repo never collides. Created here in
       *  main (the bus/facade path has no renderer to make it, unlike ipc.ts). */
      worktree?: boolean;
      /** Structured-result contract: a JSON Schema the dispatcher wants the
       *  worker's final report to carry as a fenced `wks-result` block. Not an
       *  authorization surface — it is prompt text injected into the worker and
       *  a validator run on its own output, granting the caller nothing it does
       *  not already have by writing the same words into the worker's first
       *  message. Refused (never silently dropped) when malformed or oversized;
       *  see shared/structuredResult. */
      resultSchema?: Record<string, unknown>;
      /** The agent's FIRST PROMPT, delivered by the spawn instead of by a
       *  follow-up agents.sendMessage.
       *
       *  NOT AN AUTHORIZATION SURFACE, and the evidence is the tier table:
       *  `agents.sendMessage` sits in TRIAGE (authtoken.go triageMethods) while
       *  `agents.spawn` is operator-only and deliberately absent from triage —
       *  so every caller that can reach this capability at all already holds
       *  the right to send this exact text to the session it just created. The
       *  hub's sanitizeSpawnParams passes unknown params through untouched,
       *  which is correct here: there is nothing to strip, because there is no
       *  privilege to strip it from. What it removes is the round trip and the
       *  window in between, not a check. */
      message?: string;
      /** DISPATCH TEMPLATE: the id of a library item of kind 'dispatch'
       *  (project scope of `cwd`, or global). The host renders its body —
       *  required placeholders filled from `templateParams`, hard error on any
       *  left unfilled — into the worker's first message, and applies the
       *  template's default resultSchema unless the call passes its own.
       *
       *  NOT AN AUTHORIZATION SURFACE, by construction rather than by scrub: a
       *  dispatch item is TEXT plus a default schema and carries no spawn
       *  fields at all (libraryService's parser models none), so rendering one
       *  changes nothing about the caller's authority — toolScope, cwd,
       *  worktree, skipPermissions and every clamp above still come from the
       *  CALL, exactly as they would had the caller pasted the rendered text
       *  into `message` itself. */
      template?: string;
      /** Values for the template's named placeholders. Required placeholders
       *  refuse the spawn when unfilled (see lib/dispatchTemplate.ts). */
      templateParams?: Record<string, string>;
    };
    // ── Dispatch templates: resolve + render BEFORE anything else ─────────
    // The rendered text becomes the first message; the template's default
    // resultSchema applies only when the call brought none of its own. Every
    // failure here is a REFUSED SPAWN, never a worker started on boilerplate:
    // the whole point of a required placeholder is that a template renders
    // finished-looking text, so a dispatch missing its task slot must fail
    // loudly instead of dispatching without the reasoning only the caller can
    // write (lib/dispatchTemplate.ts carries the rule).
    let message = reqMessage;
    let resultSchema = reqResultSchema;
    // Whether the first message was WRITTEN by the caller or RENDERED here — it
    // decides whether the result echoes the text back (spawnResult).
    let renderedFromTemplate = false;
    if (typeof template === 'string' && template) {
      if (typeof reqMessage === 'string' && reqMessage.trim()) {
        throw new Error(
          'agents.spawn: pass template OR message, not both — the template renders INTO the ' +
            'first message; put the task-specific text in templateParams',
        );
      }
      // Same two-step read guard as library.list (whose exposure this shares:
      // it returns a file body, here into a worker's first message): the cwd
      // against the browse roots, the derived file against the item roots.
      const templateCwd = cwd ? assertPathAllowed('agents.spawn', cwd, browseRoots()) : undefined;
      const item = libraryService
        .list(templateCwd, guardLibraryFile('agents.spawn', templateCwd))
        .find((i) => i.id === template && i.scope !== 'claude');
      if (!item) {
        throw new Error(
          `agents.spawn: no library item "${template}" in ${
            templateCwd ? `${templateCwd}/.workspacer/library or ` : ''
          }the global library`,
        );
      }
      if (item.kind !== 'dispatch') {
        throw new Error(
          `agents.spawn: library item "${template}" is kind '${item.kind}', not 'dispatch' — ` +
            'only dispatch templates render into a spawn',
        );
      }
      // {{cwd}} renders as the PROJECT directory the dispatch names — the
      // worktree (if any) is carved below, after this, and is where the worker
      // runs, not what the task is about.
      message = renderDispatchTemplate(item.body, templateParams ?? {}, {
        cwd: templateCwd ?? cwd,
      });
      renderedFromTemplate = true;
      if (resultSchema === undefined && item.resultSchema) resultSchema = item.resultSchema;
    } else if (templateParams && Object.keys(templateParams).length) {
      throw new Error('agents.spawn: templateParams was passed without a template to fill');
    }
    // SECURITY: this capability is the REMOTE/web/MCP spawn path (the local
    // desktop spawns over IPC). Driving an agent is already code execution on
    // the host, but we refuse to let a remote caller silently auto-bypass every
    // approval (`--dangerously-skip-permissions` / bypass-sandbox). Approvals
    // still surface and can be answered remotely; a YOLO agent must be started
    // locally. So `skipPermissions` is forced off here — UNLESS the hub stamped
    // `yoloGranted`, its verification that the CALLING token carries the
    // full-access grant (fleet-manager full-access mode, config
    // agents.fleetFullAccess). The stamp can't be forged: sanitizeSpawnParams
    // deletes any caller-supplied copy and re-adds it only for a verified grant
    // or the trusted host. So a granted manager's dispatched workers run
    // bypassed; every other bus spawn is still clamped.
    //
    // The two mode spellings used to be compared inline here, which made the
    // invariant look like a property of SPAWNING. It is a property of the mode,
    // and `claude.setPermissionMode` reached the same escalation on an already
    // running agent with no clamp at all — see lib/permissionBypass.ts, now the
    // single vocabulary both doors consult.
    // An OMITTED skipPermissions resolves to the config default the spawn
    // dialog pre-selects (claude.skipPermissionsDefault, or a bypass
    // defaultPermissionMode); an explicit caller value — true or false — always
    // wins. Resolved BEFORE the clamp below so a config-defaulted bypass passes
    // the SAME grant gate as an explicit request: without the hub-stamped
    // yoloGranted it is clamped identically — the operator's default never
    // escalates an ungranted token.
    const claudeCfg = configService.getConfig().claude;
    const skipDefaulted = reqSkip === undefined;
    const wantSkip = skipDefaulted
      ? claudeCfg?.skipPermissionsDefault === true ||
        permissionModeMeansBypass(claudeCfg?.defaultPermissionMode)
      : !!reqSkip;
    const yoloOK = yoloGranted === true;
    if (!yoloOK && (wantSkip || isPermissionEscalation(reqMode))) {
      console.warn(
        `[hub] agents.spawn: ignoring permission bypass ${
          skipDefaulted && wantSkip
            ? 'resolved from the config default (claude.skipPermissionsDefault / defaultPermissionMode)'
            : 'from a bus client'
        } — remote spawns never auto-bypass approvals without a hub-verified full-access grant.`,
      );
    }
    const skipPermissions = yoloOK ? wantSkip : false;
    // NO SILENT DOWNGRADES: everything this handler refuses is named to the
    // caller in the result, seeded with what the hub router already took.
    // Only an EXPLICIT request counts — a config default that never resolves is
    // the operator's setting not applying to an ungranted token, not something
    // the caller asked for and lost, and counting it would make every ordinary
    // ask-mode spawn claim it was scrubbed.
    const escalationDropped: string[] = [...(Array.isArray(hubScrubbed) ? hubScrubbed : [])];
    if (!yoloOK && !skipDefaulted && wantSkip) escalationDropped.push('skipPermissions');
    if (!yoloOK && isPermissionEscalation(reqMode)) escalationDropped.push('permissionMode');
    /** The escalation verdict every return path reports. `fullAccess` is what
     *  the session actually runs with, read AFTER the clamp rather than from
     *  the request. */
    const escalation = () => ({ fullAccess: skipPermissions, scrubbed: escalationDropped });
    // …and the same clamp on `mcpItemIds`, for the same reason and with a
    // sharper edge. A library item of kind `mcp` carries a `command`, `args` and
    // `env` verbatim into a `--mcp-config` file, and the spawn then passes
    // `--allowedTools mcp__<id>`, so the server is PRE-APPROVED and no permission
    // prompt gates it: `mcpItemIds: ['x']` is argv[0] of a host process chosen by
    // whoever wrote item `x`. And the write side cannot be closed — a bus caller
    // reaches the item through library.save OR through a plain fs.write into
    // <configDir>/library, which is a configStoreRoot by design. So the identity
    // of the SPAWNER is the only thing left to gate on: a locally-initiated spawn
    // (ipc.ts) still honours the selection, a bus one does not.
    if (mcpItemIds && mcpItemIds.length) {
      escalationDropped.push('mcpItemIds');
      console.warn(
        '[hub] agents.spawn: ignoring mcpItemIds from a bus client — an MCP server definition is argv[0] of a host process, and it is pre-approved via --allowedTools.',
      );
    }
    const busMcpItemIds = undefined;
    // …and the same for a bypass smuggled in through the PROFILE: clamping the
    // request's own fields left `profileId` as an open door (a bus caller can
    // create a profile with `--dangerously-skip-permissions` in extraArgs, or
    // reuse the user's own YOLO profile). The brain already scrubbed this; the
    // desktop path did not, so the two stacks disagreed on the invariant.
    const scrubProfileBypass = true;
    // A granted spawn keeps a bypass mode too (its whole point); otherwise an
    // escalation mode is dropped to the default, same as the skip clamp above.
    const permissionMode = !yoloOK && isPermissionEscalation(reqMode) ? undefined : reqMode;
    // Worktree isolation for a ship task (fleet-manager default): carve a fresh
    // git worktree of `cwd` and spawn the worker THERE, so parallel work on one
    // repo never collides. The IPC path does this in the renderer
    // (useAgentManager.spawnAgent); the bus/facade path has no renderer, so it
    // is done here. A soft failure (cwd not a repo, git error) falls back to
    // `cwd` with a warning rather than refusing the dispatch.
    let spawnCwd = cwd;
    if (worktree && cwd) {
      try {
        const wt = await createWorktree({
          repoCwd: cwd,
          name: label,
          // worktreeRoot is a renderer-config field (not in config_defaults), so
          // read it loosely; absent → createWorktree uses its default root.
          rootOverride: (configService.getConfig().agents as { worktreeRoot?: string } | undefined)
            ?.worktreeRoot,
          // Supplies projects[dir].worktreeSetup (+ scripts for script:<name>).
          config: configService.getConfig(),
        });
        if (wt.ok && wt.path) {
          spawnCwd = wt.path;
          if (wt.setup?.failed) {
            // The worktree is usable; the project's setup hook is not — same
            // fall-back-and-warn stance as worktree failure itself.
            console.warn(
              `[hub] agents.spawn: worktree setup "${wt.setup.failed.command}" failed (${wt.setup.failed.error}); agent starts anyway`,
            );
          }
        } else {
          console.warn(`[hub] agents.spawn: worktree for ${cwd} failed (${wt.error}); using cwd`);
        }
      } catch (err) {
        console.warn(`[hub] agents.spawn: worktree for ${cwd} threw; using cwd`, err);
      }
    }
    // Managed (Tier-2) backend — Codex / OpenCode / Pi run through claudemon's
    // adapter, not a Claude PTY. Shares the dispatch with the `claude:spawn` IPC
    // handler so this path can't silently fall back to spawning Claude (it did
    // before — `provider` was ignored here, which is why a Codex agent spawned
    // from the web/remote client came up as Claude).
    if (provider && provider !== 'claude') {
      // profileId is a Claude account concept (CLAUDE_CONFIG_DIR) with no
      // equivalent on a managed provider — same 'unsupported' classification
      // as managedSpawnOptions.ts, but ANNOUNCED rather than just never
      // reaching spawnManagedAgent, per this path's own no-silent-drop rule
      // (see the mcpItemIds warning just above).
      if (profileId) {
        escalationDropped.push('profileId');
        console.warn(
          `[hub] agents.spawn: ignoring profileId for provider "${provider}" — Claude accounts (CLAUDE_CONFIG_DIR) have no equivalent on this provider.`,
        );
      }
      const sessionId = await spawnManagedAgent({
        provider,
        cwd: spawnCwd,
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
        // The Fleet Manager pair. This branch used to hand-copy fields and
        // silently drop both — no `manager` means no isSupervisor, so a
        // codex/opencode/pi Fleet Manager dispatched over the bus (the ONLY
        // path a remote/MCP-facade caller has) never received worker-finished
        // wakes at all. See managedSpawnOptions.ts for the IPC twin of this bug.
        manager,
        fleetFullAccess,
        mcpFacade,
        toolScope,
        pluginTools,
        label,
        parentSessionId,
        cols,
        rows,
        resultSchema,
        firstMessage: message,
      });
      return spawnResult(sessionId, message, escalation(), renderedFromTemplate);
    }
    // Claude on the 'stream' transport is managed too (claudemon's headless
    // stream-json adapter, no PTY) — same shared dispatch as the IPC path so
    // the two spawn transports can't drift (standing project rule).
    const transport = reqTransport ?? claudeCfg?.transport ?? 'pty';
    if (transport === 'stream') {
      const sessionId = await spawnManagedAgent({
        provider: 'claude',
        transport: 'stream',
        cwd: spawnCwd,
        // Profile + per-spawn Library MCP servers must ride through here just
        // like on the IPC stream branch (ipc.ts) — this path used to drop both,
        // so a remote stream spawn silently ignored the chosen profile/servers.
        profileId,
        model,
        effort,
        permissionMode,
        skipPermissions,
        resumeSessionId,
        supervisor,
        // Same pair, same reason as the managed-provider branch above — a
        // Claude Fleet Manager spawned over the bus with transport 'stream'
        // must not silently come up unsupervised either.
        manager,
        fleetFullAccess,
        mcpFacade,
        toolScope,
        pluginTools,
        label,
        parentSessionId,
        mcpItemIds: busMcpItemIds,
        scrubProfileBypass,
        profileGranted: profileGranted === true,
        resultSchema,
        firstMessage: message,
      });
      return spawnResult(sessionId, message, escalation(), renderedFromTemplate);
    }
    const sessionId = await spawnClaudeAgent({
      cwd: spawnCwd,
      profileId,
      scrubProfileBypass,
      profileGranted: profileGranted === true,
      model,
      effort,
      permissionMode,
      skipPermissions,
      resumeSessionId,
      supervisor,
      manager,
      fleetFullAccess,
      mcpFacade,
      toolScope,
      pluginTools,
      label,
      parentSessionId,
      cols,
      rows,
      mcpItemIds: busMcpItemIds,
      resultSchema,
      firstMessage: message,
    });
    return spawnResult(sessionId, message, escalation(), renderedFromTemplate);
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
    // `shell` is argv[0] of a host process, taken from a bus caller. An
    // allowlist, not containment — see lib/shellAllowlist.ts.
    const resolvedShell = resolveTerminalShell(shell);
    if (resolvedShell === null) {
      throw new Error(`terminals.create: ${shell} is not one of this host's login shells`);
    }
    // One normalization, shared with the brain (lib/spawnCwd.ts explains why
    // `fs.existsSync(cwd) ? cwd : os.homedir()` had to go).
    const resolvedCwd = normalizeSpawnCwd(cwd);
    const id = await claudemonSessionClient.spawn({
      argv: [resolvedShell],
      cwd: resolvedCwd,
      cols,
      rows,
      portChannel: IPC.TERMINAL_PORT,
    });
    return { sessionId: id };
  });

  // Open a VISIBLE terminal pane in the desktop and (optionally) run a command
  // in it — the "bring up a dev server so the user can see it" path for a
  // facade agent (`open_terminal`). Unlike terminals.create (a headless,
  // driveable PTY), this asks the RENDERER to open a real terminal pane so the
  // process is watchable; the pane spawns its own PTY the normal way. Nested
  // under the calling agent's card when parentSessionId names one.
  registerCapability('terminals.open', (params: unknown) => {
    const { cwd, command, label, parentSessionId } = (params ?? {}) as {
      cwd?: string;
      command?: string;
      label?: string;
      parentSessionId?: string;
    };
    // Same cwd normalization as terminals.create; the shell is the host default
    // (the pane opens the user's login shell), so there is no argv[0] to gate
    // here — the command runs INSIDE that shell, under its own tool/PTY rules.
    const resolvedCwd = normalizeSpawnCwd(cwd);
    emitToRenderer(IPC.FACADE_OPEN_TERMINAL, {
      cwd: resolvedCwd,
      command: typeof command === 'string' ? command : undefined,
      label: typeof label === 'string' ? label : undefined,
      parentSessionId: typeof parentSessionId === 'string' ? parentSessionId : undefined,
    });
    return { ok: true };
  });

  // Surface a notification: recorded in the in-app notification center, and —
  // unless `inAppOnly` or the user disabled notifications — also shown as a
  // clickable OS notification. Clicking routes through the renderer's activate
  // path, which follows the entry's click target — the agent named by
  // `sessionId`, else the pane named by `paneType` (landing on `paneSection`
  // when it is a Settings section), else `url` externally — and brings the
  // window forward when there is none. `source` labels the sender in the
  // center ("plugin:ci").
  registerCapability('notifications.post', (params: unknown) => {
    const p = (params ?? {}) as {
      title?: string;
      body?: string;
      level?: string;
      source?: string;
      sessionId?: string;
      paneType?: string;
      paneSection?: string;
      url?: string;
      key?: string;
      silent?: boolean;
      inAppOnly?: boolean;
    };
    const title = p.title || 'workspacer';
    const body = p.body || '';
    const level = (['info', 'success', 'warn', 'error'] as const).find((l) => l === p.level);

    // Built once and shared with the OS notification's click handler below, so
    // the two surfaces can never disagree about where this thing points.
    const entry = {
      id: randomUUID(),
      level: level ?? 'info',
      title,
      body: body || undefined,
      source: typeof p.source === 'string' && p.source ? p.source : 'plugin',
      sessionId: p.sessionId,
      paneType: p.paneType,
      paneSection: p.paneSection,
      url: p.url,
      key: p.key,
      silent: p.silent === true,
      createdAt: Date.now(),
    };
    agentNotifier.postInApp(entry);

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
      // An agent/pane target goes to the renderer's own activate path, which
      // resolves both in one place — including `paneSection`, the field that
      // lands a "review this" notification on the right Settings section
      // rather than on Settings-in-general — and marks the center entry read.
      // Branching on paneType HERE instead is how that target came to be
      // accepted by this capability and then silently dropped at the click.
      if (entry.sessionId || entry.paneType) {
        agentNotifier.activateInRenderer(entry);
        return;
      }
      // A url stays on the host. It comes from whoever posted the notification
      // — a plugin, or a remote client — so it gets the same scheme check as
      // the renderer's own open-external path rather than being handed
      // straight to the OS, and it gets it whether or not a window is alive to
      // route through.
      if (entry.url) {
        void openExternalUrl(entry.url);
        return;
      }
      agentNotifier.focusWindow();
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
  //
  // SECURITY: this is agents.spawn's clamp arriving after the fact, and it was
  // missing. `mode` was validated as `typeof mode === 'string' && mode` and
  // forwarded verbatim to POST /sessions/:id/permission-mode, which accepts
  // 'bypassPermissions' on a PTY claude session (the daemon cycles Shift+Tab to
  // the bypass footer and verifies it landed) and 'yolo' on every managed
  // provider (the adapter's auto-approve flag). The sessionId is not
  // ownership-checked on either provider, so the target could be an agent the
  // LOCAL user started in ask mode — and the spawn-time clamp that refuses to
  // start a bypassing agent for a bus caller was defeated by one extra call,
  // followed by agents.sendMessage. Only the REVERSE direction (yolo→ask on a
  // session spawned in bypass) was ever gated, by claudemon, for a different
  // reason. De-escalating and neutral modes stay open: tightening is not an
  // escalation, and the remote pill needs them.
  registerCapability('claude.setPermissionMode', async (params: unknown) => {
    const { sessionId, mode } = (params ?? {}) as { sessionId?: string; mode?: string };
    if (!sessionId || typeof mode !== 'string' || !mode) {
      throw new Error('claude.setPermissionMode requires { sessionId, mode }');
    }
    // The CHECKED value is the one that travels, not the caller's variable.
    const requested = assertNoPermissionBypass('claude.setPermissionMode', mode);
    const result = await claudemonSessionClient.setPermissionMode(sessionId, requested);
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
  // Remote counterpart of the `claude:setModel` IPC handler; the provider
  // confirms the switch on the status line, but the requested model is noted
  // eagerly so the context window follows an `opus[1m]` switch immediately.
  registerCapability('claude.setModel', async (params: unknown) => {
    const { sessionId, model, effort } = (params ?? {}) as {
      sessionId?: string;
      model?: string;
      effort?: string;
    };
    if (!sessionId || (!model && !effort)) {
      throw new Error('claude.setModel requires { sessionId, model and/or effort }');
    }
    const res = await claudemonSessionClient.setModel(sessionId, model, effort);
    if (model) claudeSessionStore.noteRequestedModel(sessionId, model);
    return res;
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
    // AskUserQuestion over the adapter's control protocol). Claude PTY sessions
    // keep the keystroke path — /answer requires mode=Question, which races
    // hook mode flips (same reasoning as ClaudePane's handleAnswer). Every
    // managed (non-claude) provider — codex, opencode, pi — registers the
    // daemon's `/mcp/ask/:id` endpoint regardless of transport (codex hybrid
    // included: `start_appserver` wires it for both the headless app-server
    // and the TUI it attaches to), so a managed PTY session must ALSO go
    // structural: typing into a codex hybrid TUI composer would land as
    // ordinary chat text while the daemon's mcp_ask shim keeps the tool call
    // parked for up to its 6h timeout.
    const snap = claudeSessionStore.getSnapshot(sessionId);
    if (snap?.transport === 'stream' || (snap?.provider && snap.provider !== 'claude')) {
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
  /**
   * Re-run replay.open's containment on the session's ORIGIN cwd.
   *
   * The grant that authorized the open is not a grant that lasts. The entries
   * map is process-global and keyed by a CALLER-CHOSEN sessionId, its only
   * eviction is an explicit replay.close, and replay.* sits outside the bus's
   * per-plugin fsRoots scoping (policy.go names fs.read/fs.write/search.project,
   * not replay.*). So a worktree cut while a session was live went on serving
   * that repository's bytes after the session stopped — at which point fs.read
   * on the same directory is refused and a fresh replay.open on it is refused —
   * to any caller that knew the id, and agents.list / sessions.snapshots hand
   * ids out while being classified inert and labelled non-sensitive.
   *
   * capspec's excuse for leaving replay.read/diff/seek out of PathParam is that
   * containment here is STRUCTURAL. It is; this is the sentence that makes the
   * structure stand on a grant that is still true.
   *
   * An unknown sessionId falls through: the service's own entryOrThrow owns that
   * message, and answering it here would turn this into an existence oracle for
   * other callers' session ids.
   */
  const guardReplaySession = (cap: string, sessionId: string): void => {
    const origin = timelineReplay.originCwd(sessionId);
    if (origin === undefined) return;
    assertPathAllowed(cap, origin, workspaceRoots());
  };

  registerCapability('replay.seek', async (params: unknown) => {
    const { sessionId, ops } = (params ?? {}) as { sessionId?: string; ops?: ReplayOp[] };
    if (!sessionId) throw new Error('replay.seek requires { sessionId, ops }');
    guardReplaySession('replay.seek', sessionId);
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
    guardReplaySession('replay.read', sessionId);
    return timelineReplay.read(sessionId, p);
  });
  registerCapability('replay.diff', async (params: unknown) => {
    const { sessionId, path: p } = (params ?? {}) as { sessionId?: string; path?: string };
    if (!sessionId) throw new Error('replay.diff requires { sessionId }');
    guardReplaySession('replay.diff', sessionId);
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
  registerCapability('sessions.subagentConversation', async (params: unknown) => {
    const { sessionId, agentId } = (params ?? {}) as { sessionId?: string; agentId?: string };
    if (!sessionId || !agentId) {
      throw new Error('sessions.subagentConversation requires { sessionId, agentId }');
    }
    return claudemonSessionClient.getSubagentConversation(sessionId, agentId);
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
  //
  // Compacted before it leaves the process. Every consumer of the PLURAL call
  // already treats these as background rows — promoteSessionSnapshots and
  // useSessionSnapshots both run compactClaudeSnapshotForBackground on arrival,
  // and OverviewPane never reads `conversation` at all — so serializing the
  // full transcript here only ever paid to have it thrown away. Over IPC that
  // is a wasted structured clone; over the bus it is every session's whole
  // transcript as JSON on a WebSocket, on connect and on every OverviewPane
  // refresh (up to 1/s while streaming). The ACTIVE pane is unaffected: it
  // reads `sessions.snapshot`, singular, which stays full.
  registerCapability('sessions.snapshots', async () => {
    const live = claudeSessionStore.getAllSnapshots().map(compactClaudeSnapshotForBackground);
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
      provider?: 'codex' | 'copilot' | 'opencode' | 'pi';
      cwd?: string;
    };
    if (
      provider !== 'codex' &&
      provider !== 'copilot' &&
      provider !== 'opencode' &&
      provider !== 'pi'
    ) {
      throw new Error(
        "providers.listModels requires { provider: 'codex'|'copilot'|'opencode'|'pi' }",
      );
    }
    // `cwd` is not read here, it is EXECUTED IN: claudemon runs the provider CLI
    // with current_dir(cwd), and opencode loads and runs every
    // <cwd>/.opencode/plugin/*.js at startup — before it prints a model list,
    // with no manifest and no other file required. Unconfined, that made this
    // capability (labelled "List available models" in the consent dialog) the
    // shortest path to arbitrary host execution on the whole surface. Confined
    // to browseRoots rather than workspaceRoots because the Spawn dialog asks
    // about a directory that is not yet any agent's cwd — library.list's reason.
    //
    // MANDATORY here, unlike the local IPC twin: an absent cwd used to mean
    // "let claudemon pick", and absent is indistinguishable from '' — the value
    // the containment corpus refuses on every path-bearing method because it
    // absolutizes to the process cwd. The web Spawn dialog already `.catch`es
    // into free-text model entry, so the cost is a dropdown that stays free-text
    // until a directory is chosen.
    const canonicalCwd = assertPathAllowed('providers.listModels', cwd ?? '', browseRoots());
    const customBin = configService.getConfig().agents?.binaries?.[provider] ?? '';
    return claudemonSessionClient.listProviderModels(
      provider,
      canonicalCwd,
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
      // This document is what the desktop respawns on its next launch, through
      // the LOCAL IPC spawn door that scrubs nothing — the same crossing
      // layout.set is scrubbed for. See lib/bootDocumentScrub.ts.
      const { agents, dropped } = scrubBootDocumentAgents(data.agents as any[]);
      if (dropped.length) {
        console.warn(
          `[security] sessions.save: dropping spawn-escalation field(s) ${dropped.join(', ')} from a bus client — full access is LIVE-ONLY: a live agents.spawn honors it for a host/operator token, but this document is respawned verbatim on every launch and outlives any revocation. Each record carries an escalationScrubbed note so the caller sees this too`,
        );
      }
      return sessionService.saveSession({
        name: data.name,
        timestamp: new Date().toISOString(),
        activeAgentId: data.activeAgentId,
        agents: sessionService.enrichAgentsWithCwd(agents as any, ptyMapping),
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
  cat('layouts.save', (params: unknown) => {
    // The third copy of the boot-restore shape: a saved layout's `agents` array
    // is restored from the Layouts menu into the same respawn path.
    const input = { ...((params ?? {}) as LayoutInput) } as LayoutInput & { agents?: unknown };
    if (Array.isArray(input.agents)) {
      const { agents, dropped } = scrubBootDocumentAgents(input.agents as any[]);
      input.agents = agents;
      if (dropped.length) {
        console.warn(
          `[security] layouts.save: dropping spawn-escalation field(s) ${dropped.join(', ')} from a bus client`,
        );
      }
    }
    return layoutService.save(input as LayoutInput);
  });
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
    // SCRUB AT WRITE TIME, not only at spawn time. Everything registered with
    // cat() is a BUS entry point (the local Settings write is a separate
    // in-process IPC path, ipc.ts CLAUDE_PROFILES_ADD, and is unaffected), and
    // scrubBypassProfile used to run only on the bus SPAWN — so a bus caller
    // could persist a `configDir` (which becomes CLAUDE_CONFIG_DIR: settings.json,
    // permissions.allow and hooks, i.e. commands claude runs unprompted) plus
    // --dangerously-skip-permissions, and wait for the LOCAL user to pick that
    // profile in the New Agent dialog, where nothing scrubs. Twin of the brain's
    // registry.profilesAdd.
    const safe = scrubBypassProfile({
      configDir: configDir ?? '',
      extraArgs: extraArgs ?? [],
      mcpItemIds: mcpItemIds ?? [],
    })!;
    // mcpItemIds goes through the scrub too, and is therefore dropped. It used
    // to be forwarded PAST it — the one field the "scrubbed at write time on
    // both bus providers" record in capspec did not actually cover — and an MCP
    // server definition is `command`+`args`+`env` handed to a host process, with
    // `--allowedTools mcp__<id>` pre-approving it. SpawnAgentDialog copies a
    // profile's mcpItemIds into the spawn on selection, so a bus-planted profile
    // loaded the caller's servers into a LOCAL spawn.
    return claudeProfiles.addProfile(name, safe.configDir, safe.extraArgs, safe.mcpItemIds);
  });
  cat('claude.profiles.update', (params: unknown) => {
    const { id, updates } = (params ?? {}) as { id?: string; updates?: ProfileUpdate };
    if (!id) throw new Error('claude.profiles.update requires { id, updates }');
    // Same scrub as add: update is the other way to plant a CLAUDE_CONFIG_DIR or
    // a bypass flag on a profile the local user then picks.
    const u = { ...(updates ?? ({} as ProfileUpdate)) };
    if (u.configDir !== undefined || u.extraArgs !== undefined || u.mcpItemIds !== undefined) {
      const scrubbed = scrubBypassProfile({
        configDir: u.configDir ?? '',
        extraArgs: u.extraArgs ?? [],
        mcpItemIds: u.mcpItemIds ?? [],
      })!;
      if (u.configDir !== undefined) u.configDir = scrubbed.configDir;
      if (u.extraArgs !== undefined) u.extraArgs = scrubbed.extraArgs;
      if (u.mcpItemIds !== undefined) u.mcpItemIds = scrubbed.mcpItemIds;
    }
    return claudeProfiles.updateProfile(id, u);
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
  // Confining the cwd is not the same thing as confining what the service then
  // TOUCHES. Every read and every unlink is a path DERIVED from that cwd
  // (`<cwd>/.workspacer/library/<name>.md`, `<cwd>/.claude/skills/<id>`),
  // composed after the check and never resolved — so one symlink planted in the
  // allowed project (an ordinary permitted fs.write) read remote-token through
  // library.list and rm -rf'd the config dir through library.remove. This guard
  // is handed to the service and applied per file; a refusal skips that item
  // rather than failing the call.
  const guardLibraryFile =
    (cap: string, canonicalCwd?: string): LibraryFileGuard =>
    (filePath: string) => {
      try {
        return assertLibraryItemPath(cap, filePath, canonicalCwd);
      } catch {
        return null;
      }
    };
  // Where a library item may legitimately LIVE — deliberately narrower than the
  // allow-list the caller's `cwd` is checked against. Every file the service
  // reads, writes or unlinks is under the project it was given or in the global
  // store. Handing the per-file guard the BROWSE roots (which library.list uses
  // for its cwd, and must) made it an arbitrary home-directory reader: a
  // `<cwd>/.workspacer/library/a.md -> ~/.ssh/id_rsa` symlink canonicalizes
  // inside $HOME, passes, and comes back as an item body — while fs.read of the
  // identical path is refused. Mirrors libraryItemRoots in cmd/brain/library.go.
  const libraryItemRoots = (canonicalCwd?: string): string[] => {
    const roots = [path.join(getConfigDir(), 'library')];
    if (canonicalCwd) roots.push(canonicalCwd);
    return roots;
  };
  /**
   * The directories a library item may actually LIVE in, composed from the
   * canonical cwd and compared LEXICALLY against the already-canonical derived
   * path.
   *
   * This is the second half of the derived-path gate, and it exists because
   * libraryItemRoots alone is only as narrow as the cwd the caller named. The
   * cwd for library.list is checked against the BROWSE roots, so a caller may
   * name $HOME itself — and then "the project the caller named" IS the whole
   * home tree and the narrowing evaporates: a
   * `$HOME/.workspacer/library/a.md -> $HOME/.ssh/id_rsa` symlink (the ordinary
   * form, since git stores symlinks verbatim and a clone carries them) resolves
   * inside the root and comes back as an item body, while fs.read of the
   * identical path is refused. Requiring the RESOLVED file to sit in a library
   * directory says the thing the roots test was only approximating: a library
   * item lives in a library directory.
   *
   * Lexical on purpose. Canonicalizing these would resolve a
   * `<cwd>/.workspacer/library -> <projB>` link and hand the escape back.
   *
   * TWIN: libraryItemDirs in services/hub/cmd/brain/library.go. The brain has
   * had this half since library.* became bus-reachable; this copy had only the
   * roots, so the two providers disagreed about the same call — and the copy
   * that was wide is the one the kill switch puts back on the bus.
   */
  const libraryItemDirs = (canonicalCwd?: string): string[] => {
    const dirs: string[] = [];
    // The global store is the one entry that must be RESOLVED: the config dir
    // itself is routinely a symlinked path (XDG_CONFIG_HOME on a linked volume,
    // /var -> /private/var on macOS), so a lexical comparison against it would
    // reject every global item.
    const global = canonicalRoot(path.join(getConfigDir(), 'library'));
    if (global !== null) dirs.push(global);
    if (canonicalCwd) {
      dirs.push(path.join(canonicalCwd, '.workspacer', 'library'));
      dirs.push(path.join(canonicalCwd, '.claude'));
    }
    return dirs;
  };
  /** The whole derived-path gate for a library file: assertPathAllowed over the
   *  item roots, then the item-directory requirement above. Returns the
   *  canonical path to open (BINDING DECISION 2).
   *  TWIN: assertLibraryItemPath in services/hub/cmd/brain/library.go. */
  const assertLibraryItemPath = (cap: string, full: string, canonicalCwd?: string): string => {
    const canonical = assertPathAllowed(cap, full, libraryItemRoots(canonicalCwd));
    for (const dir of libraryItemDirs(canonicalCwd)) {
      if (containsCanonical(dir, canonical)) return canonical;
    }
    // Same non-echoing message as every other refusal on this surface.
    throw new Error(`${cap}: path is outside the allowed workspace (agent cwds + config stores)`);
  };
  cat('library.list', (params: unknown) => {
    const { cwd, kind, id } = (params ?? {}) as { cwd?: string; kind?: string; id?: string };
    // The read-only list gets browseRoots, not workspaceRoots, for the same
    // reason fs.listDir does: the New Agent dialog lists the library of the
    // directory the user is ABOUT to spawn in, which by definition isn't a live
    // agent cwd yet, and its `.catch(() => {})` would turn a refusal into a
    // silently empty project-MCP picker. Browsing the home tree to read a
    // project's own prompt files is the same exposure the picker already has;
    // writing and deleting stay on the workspace roots.
    const roots = browseRoots();
    const canonicalCwd = cwd ? assertPathAllowed('library.list', cwd, roots) : undefined;
    // The FILES get the item roots, not the browse roots: this call returns file
    // bodies, while the browse widening exists for a picker that returns names.
    // OPTIONAL narrowing, applied to the merged list (never to the read): the
    // files opened and the guard that confines them are exactly what an
    // unfiltered call opens, so `kind`/`id` can only ever REMOVE rows a caller
    // was already entitled to. It exists because the only way to learn one
    // dispatch template's placeholders used to be fetching every item's full
    // body — a filter is the difference between a ~200-byte answer and a
    // hundred-kilobyte one for a manager doing pre-spawn discovery.
    //
    // An unknown `kind` is REFUSED, not silently empty: a typo'd 'dispatchh'
    // that returns [] reads as "this library has no templates".
    //
    // An EMPTY STRING is "no filter", not "a kind named ''" — Go's omitempty
    // makes an omitted facade field arrive as "" and the brain's twin reads it
    // that way, so refusing it here would make the same call succeed on one
    // provider and fail on the other.
    if (kind && !LIBRARY_KINDS.includes(kind as LibraryKind)) {
      throw new Error(
        `library.list: unknown kind ${JSON.stringify(kind)} — one of ${LIBRARY_KINDS.join(', ')}`,
      );
    }
    return libraryService.list(canonicalCwd, guardLibraryFile('library.list', canonicalCwd), {
      kind: (kind || undefined) as LibraryKind | undefined,
      id: id || undefined,
    });
  });
  cat('library.save', (params: unknown) => {
    const input = (params ?? {}) as { cwd?: string };
    const canonicalCwd = guardLibraryCwd('library.save', input.cwd);
    // The DESTINATION is guarded too, not just the cwd it is composed from —
    // `<cwd>/.workspacer/library/<slug>.md` and `<cwd>/.claude/skills/<id>/`
    // are caller-writable locations inside an allowed root, so a symlink there
    // aimed writeFileSync at <configDir>/config.yaml while the cwd the guard
    // saw was impeccable. The brain has always guarded this derived path; this
    // copy did not, so the two providers disagreed about the same call.
    return libraryService.save(
      { ...input, cwd: canonicalCwd } as any,
      guardLibraryFile('library.save', canonicalCwd),
    );
  });
  cat('library.remove', (params: unknown) => {
    const { scope, id, cwd, kind, origin } = (params ?? {}) as {
      scope?: 'global' | 'project' | 'claude';
      id?: string;
      cwd?: string;
      kind?: 'prompt' | 'skill' | 'agent';
      origin?: ClaudeOrigin;
    };
    if (!scope || !id) throw new Error('library.remove requires { scope, id }');
    const canonicalCwd = guardLibraryCwd('library.remove', cwd);
    libraryService.remove(
      scope,
      id,
      canonicalCwd,
      kind,
      // A 'plugin:…' origin is refused by the service. The item roots would
      // refuse a plugin path anyway (they are the project plus the global
      // store), but the origin check fails LOUD instead of silently unlinking
      // nothing, which is what a caller asking to delete a plugin's skill needs
      // to hear.
      origin,
      guardLibraryFile('library.remove', canonicalCwd),
    );
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
  // ── Dismissing a finished session ──────────────────────────────────────
  //
  // SIGTERM stops a worker but its row lingers: the 30s eviction is armed by a
  // SessionEnd hook, and a killed process often emits none. The only
  // confirmation of death was sending ANOTHER signal and reading the daemon's
  // "404 no wrapper attached". This makes dismissal a verb.
  //
  // OPERATOR-ONLY by construction (agents.close is in neither scoped tier's
  // exact-name allowlist) — for the same reason claude.signal is deliberately
  // triage's ONLY stop verb: interrupting an agent is recoverable, forgetting
  // it is not. And it grants no new reach: the daemon teardown it performs is
  // claudemonSessionClient.close(), whose only escalation is the SIGTERM that
  // claude.signal already offers.
  registerCapability('agents.close', async (params: unknown) => {
    const { sessionId } = (params ?? {}) as { sessionId?: string };
    if (!sessionId) throw new Error('agents.close requires { sessionId }');
    // Throws if the session is still WORKING — checked BEFORE any teardown, so
    // a refusal leaves the worker exactly as it was.
    const before = claudeSessionStore.getAllSnapshots().find((s) => s.sessionId === sessionId);
    const result = claudeSessionStore.closeSession(sessionId);
    // Tear the daemon side down too, but only for a row that had not already
    // ended: a dismissal that left a live-but-idle wrapper attached would be a
    // lie, and re-SIGTERMing an ended session is the pointless call whose 404
    // this tool exists to replace. Best-effort — the row is gone either way,
    // which is what the caller asked for.
    let daemon: 'stopped' | 'already-ended' | 'failed' = 'already-ended';
    if (result.wasLive) {
      try {
        await claudemonSessionClient.close(sessionId);
        daemon = 'stopped';
      } catch {
        daemon = 'failed';
      }
    }
    return {
      ...result,
      daemon,
      label: before?.label,
      note: result.removed
        ? 'The session is gone from list_agents. Its desktop PANE, if the user has one open, is theirs to close.'
        : 'No such session — it had already been forgotten. Nothing to do.',
    };
  });

  // ── Threshold alerts (the alternative to polling) ──────────────────────
  //
  // Manager doctrine forbids polling — a manager looping on list_agents is a
  // hang, not monitoring — which left a worker's cost and context invisible
  // until it finished. This is how the manager honours the doctrine and still
  // sees a runaway: it asks to be told, once, and stops.
  //
  // OPERATOR-ONLY by construction, like brief.* and jobs.*: `agents.notifyWhen`
  // is not in either scoped tier's exact-name allowlist. And it is an
  // OBSERVATION, not an action — it starts nothing, changes no session, and
  // grants no reach: everything it can report (tokens, cost, idle time) is
  // already in the sessions.snapshot every VIEW token can read. What it removes
  // is the polling, not a restriction.
  registerCapability('agents.notifyWhen', (params: unknown) => {
    const { sessionId, notifySessionId, tokens, usd, idleSeconds } = (params ?? {}) as {
      sessionId?: string;
      notifySessionId?: string;
      tokens?: number;
      usd?: number;
      idleSeconds?: number;
    };
    if (!sessionId) throw new Error('agents.notifyWhen requires { sessionId }');
    // Default the recipient to the target's PARENT — the manager that
    // dispatched it is who wants to know, and it is the same routing the
    // worker-finished wake already uses. An explicit notifySessionId wins.
    const parent = claudeSessionStore
      .getAllSnapshots()
      .find((s) => s.sessionId === sessionId)?.parentSessionId;
    const watcherId = notifySessionId || parent;
    if (!watcherId) {
      throw new Error(
        'agents.notifyWhen: no notifySessionId and the target has no parent session — pass your own session id as notifySessionId',
      );
    }
    return thresholdWatcher.arm({
      sessionId,
      watcherSessionId: watcherId,
      predicate: { tokens, usd, idleSeconds },
    });
  });

  // ── A worker reporting on ITSELF (the other half of not polling) ────────
  //
  // notifyWhen above is the manager asking to be told about a NUMBER. This is
  // the worker telling its manager something only the worker knows: "the
  // approach you gave me is wrong", "phase 1 landed", "I am reading far more
  // than I expected". Before it, the only way a worker could reach its manager
  // mid-task was to be dispatched at toolScope triage/operator — tiers that
  // also hand it approve/interrupt/reply over OTHER sessions. See
  // services/progressReports.ts for the bounds (rate, lifetime cap, duplicate
  // and length refusals, all loud rather than silent).
  //
  // THE RECIPIENT IS NEVER A PARAMETER. The caller names no session but the one
  // it claims to BE (`callerSessionId`), and the host derives the recipient
  // from that session's own `parentSessionId` — so the only pair this can ever
  // connect is (a tracked session, whatever dispatched it), and a caller with
  // no parent is refused rather than routed somewhere plausible. The MCP facade
  // stamps `callerSessionId` from the per-request token record's `session:<id>`
  // label (never from caller params), and the hub bus strips it from every
  // untrusted caller, so a scoped or plugin token cannot name a session at all.
  registerCapability('agents.reportProgress', (params: unknown) => {
    const { callerSessionId, note, needsDecision } = (params ?? {}) as {
      callerSessionId?: string;
      note?: string;
      needsDecision?: boolean;
    };
    return progressReporter.report({ callerSessionId, note, needsDecision });
  });

  // ── Manager succession (the wake has to follow the fleet) ──────────────
  //
  // Every fleet wake is PARENT-KEYED: a worker-finished nudge goes to the
  // worker's own live `isSupervisor` parent, the 15-minute backstop sweeps on
  // the same field, and reportProgress above derives its recipient from it. So
  // replacing a Fleet Manager ORPHANED every dispatch it had in flight — the
  // successor could not receive their reports at all, and the workaround in the
  // /handoff skill was a manual ritual: beg each worker to leave its result on
  // disk, then reconcile a list of ids by hand.
  //
  // This is the verb that removes it. The store owns the move
  // (claudeSessionStore.reparentChildren — live sessions, not-yet-registered
  // spawns, and a finish already sitting in the coalesce window), including its
  // refusals: a successor that is unknown, ended, or not a manager cannot
  // receive a wake, and re-pointing workers at one is worse than the orphaning.
  //
  // THE CALLER IS THE SUCCESSOR, deliberately, and it names itself. The
  // verified replacement recipe destroys the outgoing manager (Terminate drops
  // the card) BEFORE the new one exists, so there is no moment at which the
  // outgoing session could name its own successor — but the successor boots
  // knowing both ids: its own is in its system prompt (mcpConfig's idNote), and
  // its predecessor's is the first line of the handoff file it reads on its
  // first turn. Doing it automatically on spawn was the alternative and was
  // rejected: with the old card and its id already gone, the host would have to
  // GUESS which dead manager a fresh one is replacing, and a wrong guess
  // silently re-points a live worker's wakes into a conversation that never
  // dispatched it — a worse failure than the one being fixed, and a silent one.
  //
  // OPERATOR-ONLY by construction, like agents.close and brief.append:
  // `agents.reparent` is in neither scoped tier's exact-name allowlist, so a
  // view scout or a phone token cannot reach it.
  // The read that makes the crash case answerable. `agents.reparent` needs a
  // `fromSessionId`, and a manager that CRASHED wrote no handoff file to read
  // one off — its row is evicted ~30 s after SessionEnd, so nothing was left of
  // it but a dangling parent id on the workers. The store now keeps a tombstone
  // for a dead MANAGER (claudeSessionStore.orphanCandidates), so the successor
  // is told which dangling parents were managers, what they were called, where
  // they worked and when they died.
  //
  // It REPORTS, it never adopts. Folding this into `agents.reparent` as a
  // no-argument "adopt whatever is orphaned" mode was the alternative and is
  // still refused: `confirmedManager` narrows the candidates but cannot say
  // which manager was YOURS, and a wrong guess silently re-points a live
  // worker's wakes into a conversation that never dispatched it. Two managers
  // can die with live children; only the caller knows which brief it was
  // handed. So the answer is a ranked list whose top row is usually obvious and
  // whose ambiguity, when there is any, is visible.
  //
  // OPERATOR-ONLY by the same construction as agents.reparent: it is in neither
  // scoped tier's exact-name allowlist. It discloses nothing new in kind — a
  // manager's label and cwd were on its own agents.list row while it lived, and
  // its children's rows are there now — only the same facts about a session
  // that has ended.
  registerCapability('agents.orphans', () => {
    const candidates = claudeSessionStore.orphanCandidates();
    const confirmed = candidates.filter((c) => c.confirmedManager);
    return {
      candidates,
      // "None" is a real and common answer (the predecessor finished its
      // dispatches, or handed over cleanly) and must not read as a failure.
      note: candidates.length
        ? `${candidates.length} dead parent(s) still have live children; ${confirmed.length} are confirmed managers. ` +
          `Pick the one you are replacing — match its label/cwd against what you were told to take over — and pass ` +
          `its sessionId as fromSessionId to adopt_workers. Adopting the wrong group re-points another manager's ` +
          `workers onto you, so do not guess between two candidates: read a worker of each first.`
        : 'Nothing is orphaned here: every live agent either has a live parent or was never dispatched by one.',
    };
  });

  registerCapability('agents.reparent', (params: unknown) => {
    const { fromSessionId, toSessionId } = (params ?? {}) as {
      fromSessionId?: string;
      toSessionId?: string;
    };
    if (!fromSessionId || !toSessionId) {
      throw new Error('agents.reparent requires { fromSessionId, toSessionId }');
    }
    const { moved, pending } = claudeSessionStore.reparentChildren(fromSessionId, toSessionId);
    const count = moved.length + pending.length;
    return {
      moved,
      pending,
      // Say which way it went, because "0 moved" is a real and useful answer —
      // the predecessor had nothing in flight — and must not read as a failure.
      note: count
        ? `${count} dispatch(es) now report to ${toSessionId}: their finished and progress wakes arrive here, not at ${fromSessionId}.`
        : `Nothing was still parented to ${fromSessionId} — it had no dispatch left in flight. Any result you are owed is on disk or in its transcript.`,
    };
  });

  // ── Project briefs ─────────────────────────────────────────────────────
  //
  // The atomic inspect-then-edit primitive. Before it, every brief update was
  // fs.read + fs.write with an unbounded window between them — over a file a
  // manager and its workers write at the same moment, because the trigger for
  // both is the same worker finishing. See services/briefService for the
  // guarantee (strictly additive, locked, compare-and-swapped).
  //
  // OPERATOR-ONLY by construction, like jobs.*: `brief.append` matches no
  // scoped tier's allowlist (authtoken viewMethods/triageMethods are exact
  // names), so a view scout or a phone token cannot reach it.
  //
  // And it WIDENS NOTHING. It is path-scoped in capspec on `project` and takes
  // the SAME workspaceRoots() fs.write takes, so it reaches no directory
  // fs.write could not already write — and strictly less within one, because
  // the caller never names a file: the basename is composed here. In the
  // deployment that wants it, that root set is exactly right — the Fleet
  // Manager's cwd is the projects' common parent, and containment is by
  // subtree, so every project under it is already in the set. No live manager,
  // no brief.append; that is the correct answer, not a gap to widen for.
  //
  // APPEND-FROM-RESULT (the optional `sessionId` / `result` params). A brief
  // line is a sentence of JUDGEMENT plus a run of MECHANICAL FACTS, and the
  // manager was retyping both — spending tokens on the half the worker already
  // reported verbatim in its `wks-result` block, and MISTRANSCRIBING it (a live
  // manager wrote `session:6a-round2` into a brief, a nickname where a session
  // id belongs, and the dead link had to be repaired by hand). With either
  // param present the host composes the line: date, the manager's sentence,
  // the facts, the validated `session:<short id>`. `line` stays REQUIRED and
  // still carries the judgement — see lib/briefResultLine for why a result
  // object alone must never become a line.
  //
  // IT WIDENS NOTHING, which is the only question the tiering asks: same
  // method, same operator-only reachability, same `project` path scope, same
  // composed-and-guarded brief path. The new params are strings and JSON that
  // reach only a pure string renderer; a view or triage token still cannot call
  // brief.append at all, so there is no route by which composing a line for a
  // caller grants that caller anything it did not already hold.
  registerCapability('brief.append', (params: unknown) => {
    const { project, section, line, sessionId, result } = (params ?? {}) as {
      project?: string;
      section?: string;
      line?: string;
      sessionId?: unknown;
      result?: unknown;
    };
    if (!project || isAsciiBlank(project)) throw new Error('brief.append requires { project }');
    if (line === undefined) throw new Error('brief.append requires { line }');
    // Guard the PROJECT directory (the caller's only path input, and the one
    // capspec declares), then compose the brief path under the CANONICAL root
    // the guard returned — resolving the guard's answer rather than the
    // caller's string is what stops a symlinked project dir from being
    // re-interpreted after the check.
    const dir = assertPathAllowed('brief.append', project, workspaceRoots());
    // AND GUARD THE COMPOSED PATH, which the line above does not cover.
    //
    // "The caller cannot name a file" bounds the BASENAME and says nothing
    // about the DIRECTORIES composed on the way to it. `project` can be a
    // perfectly legitimate allowed directory — an agent's own cwd — while
    // `<project>/.workspacer` is a SYMLINK pointing out of every workspace
    // root: the guard above resolves the directory it was handed and answers
    // yes, truthfully, about a path that is not the one being written.
    //
    // Found by construction in the Go port of this capability
    // (services/hub/cmd/brain/brief.go), whose
    // TestBriefAppendDoesNotFollowASymlinkedWorkspacerDirectory caught the
    // identical shape here. assertPathAllowed resolves per component and
    // tolerates a leaf that does not exist yet, which is what fs.write already
    // relies on, so the composed path can be asserted before it is created.
    const briefPath = assertPathAllowed('brief.append', briefPathFor(dir), workspaceRoots());
    // The composed line when the caller asked for one, the caller's own line
    // otherwise — byte for byte, so plain brief_append is untouched by this.
    const text = hasResultParams({ sessionId, result })
      ? composeResultLine({ significance: line, sessionId, result })
      : line;
    return appendBriefLine(briefPath, parseBriefSection(section), text);
  });

  // The READ-ONLY half of brief maintenance, and the counterweight to the fact
  // that everything else here can only ADD: which `## Now` lines are talking
  // about workers that no longer exist.
  //
  // Every brief in this fleet records the same lesson in its own words — a Now
  // line does not remove itself when its worker dies — because the wake that
  // would remind the manager to move the line is the same wake handing it a
  // result to act on, and the line loses. The cost lands on the NEXT manager,
  // which reads four dispatch lines, believes four workers are running, and
  // goes looking for sessions that ended days ago.
  //
  // IT FLAGS AND NEVER WRITES. Not a limitation — doctrine. The brief is the
  // user's document and their own edits are authoritative, which is why every
  // write path here is additive (briefService) or move-only (briefBoardService).
  // A checker that pruned would be the single component able to destroy
  // hand-written prose on the strength of a heuristic, and the heuristic is
  // wrong sometimes. So it returns a report and the manager decides.
  //
  // Same tier story and the SAME confinement as its two siblings: operator-only
  // by construction (brief.* matches no scoped tier's exact-name allowlist),
  // path-scoped on `project`, both path components composed and guarded here.
  // It reads strictly less than fs.read already reads within the same root.
  registerCapability('brief.check', (params: unknown) => {
    const { project } = (params ?? {}) as { project?: string };
    if (!project || isAsciiBlank(project)) throw new Error('brief.check requires { project }');
    const dir = assertPathAllowed('brief.check', project, workspaceRoots());
    // The composed path is guarded for the reason brief.append's own comment
    // spells out: `project` can be an allowed directory while
    // `<project>/.workspacer` is a symlink pointing out of every root.
    const briefPath = assertPathAllowed('brief.check', briefPathFor(dir), workspaceRoots());
    let content = '';
    try {
      content = fs.readFileSync(briefPath, 'utf-8');
    } catch (err) {
      // A project with no brief is not an error — it has no stale Now lines,
      // which is the honest answer and the one a manager can act on.
      if ((err as NodeJS.ErrnoException).code !== 'ENOENT') throw err;
    }
    return checkNowSection(
      content,
      liveSessionIds(claudeSessionStore.getAllSnapshots()),
      briefPath,
    );
  });

  // The other half of the same document: trim a section by moving its OLDEST
  // entries into `<project>/.workspacer/brief.archive.md`. This is the Board's
  // drag-to-Archive move called as a capability, not a second implementation of
  // it: the entries are spliced out whole and appended to the archive verbatim,
  // under the SAME lock brief.append takes.
  //
  // It exists because /checkpoint had no way to call it. The skill could only
  // append, so the model did the trim in shell instead, and one morning of that
  // left three differently worded headings in the archive and a litter of .bak
  // files beside the brief. The judgement stays with the model (which section,
  // how many to keep); only the mechanics move into code.
  //
  // Same tier story and the SAME confinement as brief.append: operator-only by
  // construction, path-scoped on `project`, and the caller never names a file.
  // It cannot name an entry either, so it can only ever take the oldest end of
  // one section, and everything it takes is still on disk in the archive.
  registerCapability('brief.archive', (params: unknown) => {
    const { project, section, count, keep } = (params ?? {}) as {
      project?: string;
      section?: string;
      count?: number;
      keep?: number;
    };
    if (!project || isAsciiBlank(project)) throw new Error('brief.archive requires { project }');
    const dir = assertPathAllowed('brief.archive', project, workspaceRoots());
    // Both composed paths, for the reason spelled out on brief.append above: a
    // symlinked `<project>/.workspacer` escapes a guard that only ever resolved
    // `project`. This one moves entries BETWEEN the two files, so an unguarded
    // compose could both read a brief and write an archive outside every root.
    // The guarded answers are discarded rather than threaded through
    // archiveOldestEntries — which takes the directory and composes both
    // basenames itself — because the assertion is the point: it throws on an
    // escape, and past it the directory really does contain both files.
    assertPathAllowed('brief.archive', briefPathFor(dir), workspaceRoots());
    assertPathAllowed(
      'brief.archive',
      path.join(dir, '.workspacer', 'brief.archive.md'),
      workspaceRoots(),
    );
    return archiveOldestEntries({ dir, section: parseBriefSection(section), count, keep });
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
    const requested = p && !isAsciiBlank(p) ? p : home;
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
        // Byte-wise: `sort.Strings(dirs)` in the Go twin (cmd/brain/fsops.go).
        .sort(byteCompare);
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
  /**
   * Anchor a caller-supplied pathspec on the work-tree root git will actually
   * resolve it in, hold the result to every root set in `rootSets`, and return
   * it in the root-relative form git wants.
   *
   * The DERIVED work-tree root is the thing to be careful about here. gitService
   * runs every command from `rev-parse --show-toplevel` (see its header), and
   * that directory comes out of git AFTER the cwd guard — nothing ever checked
   * it against the allow-list. So resolving a pathspec against the caller's
   * `cwd` would check a different file than git opens whenever the agent cwd is
   * a subdirectory (the ordinary monorepo case), and treating the root as
   * trusted turns "a pathspec inside the confined repo" into "any path inside a
   * repository that merely CONTAINS an allowed directory".
   *
   * Concatenation, not path.resolve/path.join: those collapse a `link/..` pair
   * textually before any symlink is read, which is precisely the check-path /
   * opened-path split the component walk exists to close. The walk inside
   * assertPathAllowed does the resolving.
   */
  const anchorGitPathspec = async (
    cap: string,
    canonicalCwd: string,
    filePath: string,
    extraRootSets: string[][] = [],
  ): Promise<string> => {
    const root = (await git.workRoot(canonicalCwd)) ?? canonicalCwd;
    const anchored = path.isAbsolute(filePath)
      ? filePath
      : root.endsWith(path.sep)
        ? root + filePath
        : root + path.sep + filePath;
    // Always: inside the repository git is about to resolve the pathspec in.
    const canonicalFile = assertPathAllowed(cap, anchored, [root]);
    // …plus whatever narrower boundary the particular leg demands. Each set is a
    // separate assertion, so a caller has to satisfy ALL of them.
    for (const roots of extraRootSets) assertPathAllowed(cap, anchored, roots);
    // git runs from the work-tree root, so it receives the validated path
    // expressed from that root: the operand is a function of the CANONICAL path,
    // never of the caller's string. (Root-relative is what git wants for a
    // pathspec; the absolute form is the fallback for the degenerate "the path
    // IS the root" case.)
    const canonicalRootPath = canonicalRoot(root) ?? root;
    return path.relative(canonicalRootPath, canonicalFile) || canonicalFile;
  };
  /**
   * The pathspec that means "everything this call is allowed to touch" for a
   * git.* mutation that was given no `path` — the already-guarded cwd, expressed
   * from the work-tree root.
   *
   * Without it, `git.stage {cwd}` with no path runs `git add -A` FROM THE ROOT,
   * which stages every file in a repository that merely contains the allowed
   * cwd: an agent cwd of <repo>/proj staged <repo>/prod-key.pem, and a path-less
   * `git.diff {staged: true}` then rendered each newly-indexed file as an
   * all-added diff with full content — a file fs.read, fs.watch and
   * git.diff{untracked} all refuse for the same caller. Neither call is a path
   * escape on its own; the pair is, so the staging leg is bounded here.
   */
  const cwdPathspec = async (cap: string, canonicalCwd: string): Promise<string> => {
    const root = (await git.workRoot(canonicalCwd)) ?? canonicalCwd;
    // Proves the cwd really is at-or-inside the derived root before path.relative
    // is trusted to describe it (a `..` result would be a pathspec pointing OUT
    // of the repo, i.e. the escape this helper exists to close).
    const checked = assertPathAllowed(cap, canonicalCwd, [root]);
    const canonicalRootPath = canonicalRoot(root) ?? root;
    return path.relative(canonicalRootPath, checked) || '.';
  };
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
    //
    // …but the "concedes nothing a path-less git.diff doesn't already hand
    // over" argument is only true for a TRACKED pathspec. With `untracked`,
    // git.diff runs `git diff --no-index -- /dev/null <path>`, which renders
    // ANY readable file as an all-added diff — gitignored, untracked, and
    // tracked-but-unmodified files alike, none of which appear in a path-less
    // diff (verified: it returns ""). That turns the derived work-tree root —
    // a directory nothing ever checked against the allow-list — into an
    // arbitrary reader: an agent cwd of <repo>/frontend read <repo>/backend/
    // .env, and a $HOME that happens to be a dotfiles repo read ~/.ssh/id_rsa,
    // both of which fs.read and fs.watch refuse for the same caller. So this
    // one leg is held to the ordinary workspace roots as well.
    const operand = filePath
      ? await anchorGitPathspec(
          'git.diff',
          canonicalCwd,
          filePath,
          untracked ? [workspaceRoots()] : [],
        )
      : filePath;
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
  // git.stage is the WRITE half of an exfiltration the read half alone does not
  // achieve, which is why the guard is here and not only on git.diff.
  //
  // `git add` runs from the DERIVED work-tree root, and `path` used to travel to
  // gitService with no check at all: a root-relative pathspec
  // (`backend/prod-key.pem`) — or NO pathspec, which meant `git add -A` over the
  // whole repository — put files outside every allowed root into the index. A
  // path-less `git.diff {staged: true}` then renders each of them as an
  // all-added diff with FULL CONTENT, because they are not in HEAD; git.commit
  // persists it, git.commitDiff hands it back, git.push publishes it. Every one
  // of those files is refused to the same caller by fs.read, fs.watch and the
  // (already fixed) `git.diff {path, untracked}` leg.
  //
  // So the staging leg gets the boundary the untracked-diff leg got — the
  // ordinary workspace roots, not merely "inside the repo" — and the path-less
  // form is bounded to the guarded cwd instead of the root. That narrows a
  // remote "Stage All" in a monorepo whose agent cwd is a subdirectory to that
  // subdirectory; it is the same trade the untracked leg already made, and the
  // local desktop IPC path (the trusted user reviewing their own repo) is
  // untouched.
  registerCapability('git.stage', async (params: unknown) => {
    const { cwd, path: filePath } = (params ?? {}) as { cwd?: string; path?: string };
    if (!cwd) throw new Error('git.stage requires { cwd }');
    const canonicalCwd = guardGitCwd('git.stage', cwd);
    const operand = filePath
      ? await anchorGitPathspec('git.stage', canonicalCwd, filePath, [workspaceRoots()])
      : await cwdPathspec('git.stage', canonicalCwd);
    const output = await git.stage(canonicalCwd, operand);
    return { ok: true, output };
  });
  // The same two holes, pointed the other way. Unstaging does not hand content
  // back, but `git reset -q HEAD` from the root drops the index for a whole
  // repository the caller was granted one directory of — and the decision on
  // record for this param ("git resolves it relative to the work-tree root the
  // guard returned") was as untrue here as it was for git.stage.
  registerCapability('git.unstage', async (params: unknown) => {
    const { cwd, path: filePath } = (params ?? {}) as { cwd?: string; path?: string };
    if (!cwd) throw new Error('git.unstage requires { cwd }');
    const canonicalCwd = guardGitCwd('git.unstage', cwd);
    const operand = filePath
      ? await anchorGitPathspec('git.unstage', canonicalCwd, filePath, [workspaceRoots()])
      : await cwdPathspec('git.unstage', canonicalCwd);
    const output = await git.unstage(canonicalCwd, operand);
    return { ok: true, output };
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
