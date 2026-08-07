import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';
import { claudemonSessionClient } from './claudemonSessionClient';
import { getConfigDir } from './configService';
import { atomicWriteFileSync } from '../lib/atomicWriteFile';
import { slugSession } from '../lib/fileUtils';
import { asString, byteCompare, trimSuffix } from '../lib/providerParity';
import { canonicalizePath, isWithin, resolveStoreEntry } from '../lib/pathConfinement';
import { SESSION_SCHEMA_VERSION } from '../shared/sessionSchema';

interface SessionPaneData {
  id: string;
  type: string;
  title: string;
  width: number;
  widthOverride?: number;
  shell?: string;
  cwd?: string;
  url?: string;
}

interface SessionTabData {
  id: string;
  title: string;
  panes: SessionPaneData[];
  activePaneId: string;
  /** Epoch ms of the tab's last activity (focus / creation / split). */
  lastActiveAt?: number;
}

interface SessionAgentData {
  id: string;
  name: string;
  global?: boolean;
  cwd: string;
  profileId?: string;
  model?: string;
  skipPermissions?: boolean;
  sessionId?: string;
  tabs: SessionTabData[];
  activeTabId: string;
}

interface SessionData {
  name: string;
  timestamp: string;
  // Agent-centric layout (current): a roster of agent workspaces, each with tabs.
  activeAgentId?: string;
  agents?: SessionAgentData[];
  // Legacy flat layout — a single set of tabs/panes — kept for backward compat.
  activeTabId?: string;
  tabs?: SessionTabData[];
  activePaneId?: string;
  panes?: SessionPaneData[];
}

interface SessionListEntry {
  name: string;
  filename: string;
  timestamp: string;
  paneCount: number;
  agentCount: number;
}

function getSessionsDir(): string {
  return path.join(getConfigDir(), 'sessions');
}

/**
 * Resolve a caller-supplied session `filename` against the sessions dir and
 * confine it there. `loadSession` / `deleteSession` are reachable from the hub
 * bus (`sessions.load` / `sessions.delete`) and therefore from a remote client,
 * so a `filename` must not read or delete outside the sessions directory.
 *
 * This is a FOURTH copy of path containment, and it used to be the odd one out
 * twice over. It accepted any MULTI-SEGMENT name (`path.resolve` + `startsWith`,
 * a purely lexical check) where the Go twin — cmd/brain stores.go
 * `sessionFilePath`, the copy that answers under the default catalog delegation
 * — requires a bare basename; so `sessions.load('esc/loot.yaml')` returned a
 * file outside the sessions dir and `sessions.delete` unlinked it, through a
 * directory symlink, while the brain refused the identical input. And a lexical
 * check is exactly the algorithm BINDING DECISION 2 exists to reject: it
 * collapses `..` before any symlink is read, so the checked path and the opened
 * path are two different files.
 *
 * Both halves are now the rule, in both copies:
 *   1. a bare basename, never a path — `.`/`..`/anything with a separator is
 *      refused rather than resolved, matching sessionFilePath;
 *   2. canonicalize per component and require the RESULT to sit inside the
 *      sessions dir, so a symlink named like a session file cannot point out of
 *      it, and the caller gets back the string that is actually opened.
 *
 * The contract corpus pins both sides: contracts/path-containment-cases.json
 * `sessionFilenames`, loaded here and by cmd/brain/stores_test.go.
 */
function resolveWithinSessionsDir(filename: string): string {
  const dir = getSessionsDir();
  const refuse = (): never => {
    throw new Error(`session filename escapes the sessions directory: ${filename}`);
  };
  if (!filename || filename === '.' || filename === '..' || filename !== path.basename(filename)) {
    refuse();
  }
  let canonical: string;
  try {
    canonical = canonicalizePath(path.join(dir, filename));
  } catch {
    refuse(); // unverifiable → deny, same posture as the fs.* guard
  }
  if (!isWithin(canonical!, dir)) refuse();
  return canonical!;
}

const sanitizeFilename = slugSession;

function getTerminalCwd(sessionId: string): string | undefined {
  // claudemon owns the PTY in a separate process, so we can't /proc-walk it.
  // Fall back to the cwd we spawned with (claudemonSessionClient tracks it).
  return claudemonSessionClient.getCwd(sessionId);
}

class SessionService {
  private ensureDir(): void {
    fs.mkdirSync(getSessionsDir(), { recursive: true });
  }

  listSessions(): SessionListEntry[] {
    this.ensureDir();
    const dir = getSessionsDir();

    try {
      const files = fs.readdirSync(dir).filter((f) => f.endsWith('.yaml'));
      const entries: SessionListEntry[] = [];

      for (const file of files) {
        // Same rule the caller-supplied `filename` gets above, applied to a name
        // this method DERIVED from a readdir: a symlink named like a session is
        // a legal entry, the sessions dir is bus-writable, and listing must not
        // become a reader of whatever it points at. Twin: stores.go
        // storeEntryPath.
        const full = resolveStoreEntry(dir, file);
        if (full === null) continue;
        try {
          const data = fs.readFileSync(full, 'utf-8');
          const session = yaml.load(data) as SessionData;
          const agents = session.agents ?? [];
          const paneCount =
            agents.length > 0
              ? agents.reduce(
                  (n, a) => n + (a.tabs ?? []).reduce((m, t) => m + (t.panes?.length || 0), 0),
                  0,
                )
              : (session.tabs?.reduce((m, t) => m + (t.panes?.length || 0), 0) ??
                session.panes?.length ??
                0);
          entries.push({
            // Coerced, and TrimSuffix rather than replace(): `str()` and
            // `strings.TrimSuffix` are what the Go twin uses, and `replace`
            // removes the FIRST occurrence anywhere in the name.
            name: asString(session.name) || trimSuffix(file, '.yaml'),
            filename: file,
            timestamp: asString(session.timestamp),
            paneCount,
            agentCount: agents.filter((a) => !a.global).length,
          });
        } catch {
          // Skip malformed session files
        }
      }

      // Sort by timestamp descending (most recent first), byte-wise over an
      // already-coerced string — matching `out[i].Timestamp > out[j].Timestamp`
      // in cmd/brain/stores.go. localeCompare is a method, so a non-string YAML
      // scalar threw inside the comparator and the catch below returned an EMPTY
      // LIST, taking every well-formed session with it while the brain listed
      // them all. <configDir>/sessions is a configStoreRoot, so writing that file
      // is an ordinary permitted fs.write.
      entries.sort((a, b) => byteCompare(b.timestamp, a.timestamp));
      return entries;
    } catch {
      return [];
    }
  }

  loadSession(filename: string): SessionData | null {
    // Containment first, outside the try: a traversal attempt is a hard reject
    // that must surface to the caller, not be swallowed into a null "not found".
    const filePath = resolveWithinSessionsDir(filename);
    try {
      const data = fs.readFileSync(filePath, 'utf-8');
      return yaml.load(data) as SessionData;
    } catch {
      return null;
    }
  }

  saveSession(data: SessionData): string {
    this.ensureDir();
    const base = sanitizeFilename(data.name);
    // Two distinct session names can slug to the same file (e.g. 'Feature: Auth'
    // and 'Feature Auth' both -> feature-auth.yaml). Writing blindly would let the
    // second session clobber the first (silent data loss). Reuse the file only
    // when it already holds THIS session (same name) — which keeps autosaves
    // stable — otherwise pick the next free numeric suffix.
    // Through the SAME resolver as load and delete. This leg used to be a bare
    // path.join, which is the third of three paths that must agree about what a
    // legal session file is — capspec's own record says so ("re-checked by the
    // same resolver") and the Go twin honours it literally (stores.go
    // saveSavedSession → sessionFilePath). Two consequences of the join:
    // the collision loop's readFileSync followed a symlinked entry OUT of the
    // store, and the returned filename then depended on that file's `name` field
    // (my-session.yaml vs my-session-2.yaml) — a bus-visible content oracle on a
    // file sessions.load refuses outright; and the write then replaced the entry.
    let filename = base + '.yaml';
    let filePath = resolveWithinSessionsDir(filename);
    for (let i = 2; fs.existsSync(filePath); i++) {
      let existingName: string | undefined;
      try {
        existingName = (yaml.load(fs.readFileSync(filePath, 'utf-8')) as SessionData)?.name;
      } catch {
        // Malformed file — don't overwrite data we can't identify.
      }
      if (existingName === data.name) break;
      filename = `${base}-${i}.yaml`;
      filePath = resolveWithinSessionsDir(filename);
    }
    // Stamp the format version so a future build can tell "I don't understand
    // this" from "this is empty" — see contracts/session-schema.json.
    const yamlStr = yaml.dump(
      { schemaVersion: SESSION_SCHEMA_VERSION, ...data },
      { lineWidth: -1 },
    );
    atomicWriteFileSync(filePath, yamlStr);
    return filename;
  }

  deleteSession(filename: string): void {
    // Containment first, outside the try: a traversal attempt must reject loudly
    // rather than be mistaken for a "file didn't exist" no-op.
    const filePath = resolveWithinSessionsDir(filename);
    try {
      fs.unlinkSync(filePath);
    } catch {
      // Ignore if file doesn't exist
    }
  }

  enrichPanesWithCwd(
    panes: SessionPaneData[],
    ptyMapping: Record<string, string>,
  ): SessionPaneData[] {
    return panes
      .filter((p) => p.type !== 'settings')
      .map((pane) => {
        if (pane.type === 'terminal' && ptyMapping[pane.id]) {
          const cwd = getTerminalCwd(ptyMapping[pane.id]);
          return { ...pane, cwd: cwd || pane.cwd };
        }
        return pane;
      });
  }

  /** Enrich every pane inside an agent roster's tabs with its terminal cwd. */
  enrichAgentsWithCwd(
    agents: SessionAgentData[],
    ptyMapping: Record<string, string>,
  ): SessionAgentData[] {
    return agents.map((agent) => ({
      ...agent,
      tabs: (agent.tabs ?? []).map((tab) => ({
        ...tab,
        panes: this.enrichPanesWithCwd(tab.panes ?? [], ptyMapping),
      })),
    }));
  }
}

export const sessionService = new SessionService();
