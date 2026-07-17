// External tool dependencies — the single source of truth for which system
// binaries Workspacer's features shell out to, whether each is currently
// available, and what to tell the user when one is missing.
//
// The UI asks via the `tools:status` IPC (renderer hook: useToolStatus) at the
// point a dependent feature is opened — e.g. the Review pane needs git — and
// shows the registry's install hint instead of a raw ENOENT. Checks scan PATH
// directly (a GUI-launched Electron inherits the login PATH; same approach as
// claudeResolver) and are cached until an explicit re-check, since PATH only
// changes when the user installs something.
//
// NOT listed here: ripgrep (bundled via @vscode/ripgrep — see searchService),
// and per-plugin runtimes (the examples gallery labels those individually).
import * as fs from 'fs';
import * as path from 'path';
import { execFile } from 'child_process';

export type ToolId = 'git' | 'claude' | 'codex' | 'opencode' | 'pi' | 'tailscale';

export interface ToolSpec {
  id: ToolId;
  /** Human name for dialogs ("Git"). */
  label: string;
  /** Binary name looked up on PATH (platform extensions handled). */
  bin: string;
  /** Features that stop working without it — shown in the missing-tool notice. */
  features: string[];
  /** One-line install hint for the current platform. */
  install: string;
}

export interface ToolStatus extends ToolSpec {
  available: boolean;
  /** Resolved absolute path when available. */
  path?: string;
}

function gitInstallHint(): string {
  switch (process.platform) {
    case 'darwin':
      return 'Install with `xcode-select --install` (or `brew install git`)';
    case 'win32':
      return 'Install with `winget install Git.Git` — https://git-scm.com/downloads';
    default:
      return 'Install git with your package manager (e.g. `pacman -S git`, `apt install git`)';
  }
}

export const TOOL_REGISTRY: ToolSpec[] = [
  {
    id: 'git',
    label: 'Git',
    bin: 'git',
    features: [
      'Review changes pane',
      'Per-turn changed-files cards',
      'Branch display in the status bar',
      'Worktree isolation',
    ],
    install: gitInstallHint(),
  },
  {
    id: 'claude',
    label: 'Claude Code',
    bin: 'claude',
    features: ['Claude agents'],
    install: 'Install with `npm install -g @anthropic-ai/claude-code`',
  },
  {
    id: 'codex',
    label: 'Codex CLI',
    bin: 'codex',
    features: ['Codex agents'],
    install: 'Install with `npm install -g @openai/codex`',
  },
  {
    id: 'opencode',
    label: 'OpenCode',
    bin: 'opencode',
    features: ['OpenCode agents'],
    install: 'Install from https://opencode.ai',
  },
  {
    id: 'pi',
    label: 'Pi',
    bin: 'pi',
    features: ['Pi agents'],
    install: 'Install with `npm install -g @earendil-works/pi-coding-agent`',
  },
  {
    id: 'tailscale',
    label: 'Tailscale',
    bin: 'tailscale',
    features: ['Remote share (HTTPS via tailscale serve)'],
    install: 'Install from https://tailscale.com/download',
  },
];

/** Windows resolves bare names through PATHEXT; elsewhere the name is exact. */
function candidateNames(bin: string): string[] {
  if (process.platform !== 'win32') return [bin];
  const exts = (process.env.PATHEXT || '.COM;.EXE;.BAT;.CMD').split(';').filter(Boolean);
  return [...exts.map((e) => bin + e.toLowerCase()), bin];
}

/** First PATH entry containing the binary, or undefined. */
export function resolveOnPath(bin: string): string | undefined {
  const dirs = (process.env.PATH || '').split(path.delimiter).filter(Boolean);
  for (const dir of dirs) {
    for (const name of candidateNames(bin)) {
      const full = path.join(dir, name);
      try {
        if (fs.existsSync(full) && fs.statSync(full).isFile()) return full;
      } catch {
        /* unreadable PATH entry — skip */
      }
    }
  }
  return undefined;
}

let cache: Map<ToolId, ToolStatus> | null = null;

function checkAll(): Map<ToolId, ToolStatus> {
  const map = new Map<ToolId, ToolStatus>();
  for (const spec of TOOL_REGISTRY) {
    const resolved = resolveOnPath(spec.bin);
    map.set(spec.id, { ...spec, available: !!resolved, path: resolved });
  }
  return map;
}

/**
 * Status of every registered tool. Cached after the first call; pass
 * `force` (the UI's "Check again" button) after the user installs something.
 */
export function toolsStatus(force = false): ToolStatus[] {
  if (!cache || force) cache = checkAll();
  return [...cache.values()];
}

/** Best-effort `<bin> --version` for diagnostics surfaces. */
export function toolVersion(id: ToolId): Promise<string | null> {
  const status = toolsStatus().find((t) => t.id === id);
  if (!status?.available || !status.path) return Promise.resolve(null);
  return new Promise((resolve) => {
    execFile(status.path!, ['--version'], { timeout: 3000 }, (err, stdout) => {
      resolve(err ? null : stdout.trim().split('\n')[0] || null);
    });
  });
}
