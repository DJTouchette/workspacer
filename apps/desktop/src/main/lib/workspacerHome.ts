/**
 * `~/.workspacer` — the app's own home directory, and the creator of it.
 *
 * It lived in services/supervisorSkill.ts until the fleet-supervisor role was
 * removed; despite the name, nothing about it is supervisor-specific. The
 * directory holds the fleet brief, model-rates.json, handoffs, worktrees and
 * fonts, and it is the cwd for agents that are about the fleet rather than any
 * one project (the Fleet Manager, the Guide, "Ask the Fleet").
 *
 * The exported name is unchanged on purpose: `app.supervisorHome` is a live IPC
 * channel and hub-bus capability with a Go twin, and renaming it is a
 * whole-stack change, not part of this one.
 */
import * as os from 'os';
import * as path from 'path';
import * as fs from 'fs';

/** What ~/.workspacer/README.md said before this directory grew settings in it.
 *  Kept verbatim so an untouched copy can be upgraded in place; a README the
 *  user has edited is left exactly as they left it. */
const LEGACY_HOME_README =
  '# Workspacer supervisor home\n\n' +
  'This directory is the working directory for fleet **supervisor** agents\n' +
  'spawned from Workspacer (Ask the Fleet). They coordinate your other\n' +
  'Claude Code agents via the workspacer MCP tools and use this folder as a\n' +
  'neutral scratch space — notes, digests, etc. Safe to delete; it is\n' +
  'recreated on the next supervisor spawn.\n';

/** The README written into ~/.workspacer. It stopped being true when this
 *  directory started holding things the user is meant to keep: model-rates.json
 *  is read live by the daemon, scripts/ is what job specs point at, brief.md is
 *  the fleet's memory between sessions. "Safe to delete" was fine for a scratch
 *  space and is wrong for a settings directory, which is why it is gone. */
const HOME_README =
  '# Workspacer home\n\n' +
  'Your user-global work{spacer} working directory. Fleet **manager** agents\n' +
  'run here rather than in some random project, so their notes and digests\n' +
  'land here.\n\n' +
  'It also holds things you are meant to keep and to edit:\n\n' +
  "- `brief.md` — the fleet brief, the manager's memory between sessions\n" +
  '- `model-rates.json` — per-model price overrides, re-read while running\n' +
  '- `scripts/` — scripts your jobs point at, e.g.\n' +
  '  `"command": "~/.workspacer/scripts/nightly-tests"`\n' +
  '- `worktrees/`, `handoffs/`, `fonts/` — created on demand\n\n' +
  'Loose notes in here are scratch and yours to delete. The files above are not:\n' +
  'they are settings, and deleting them loses what you put in them.\n\n' +
  'Job specs are NOT here. They belong to the hub process and live beside your\n' +
  'config dir, at `~/.config/workspacer-hub/jobs.json`.\n';

/**
 * The user's work{spacer} home directory: `~/.workspacer`. An agent that works
 * across the whole fleet rather than in one project opens here — a stable,
 * neutral place — instead of landing in some random agent's repo. Created (with
 * a short README) on first use. Best-effort: if creation fails we fall back to
 * the home dir. Shared by both spawn paths (ipc.ts and hubCapabilities.ts).
 */
export function ensureSupervisorHome(): string {
  const dir = path.join(os.homedir(), '.workspacer');
  try {
    fs.mkdirSync(dir, { recursive: true });
    const readme = path.join(dir, 'README.md');
    let current = '';
    try {
      current = fs.readFileSync(readme, 'utf8');
    } catch {
      /* not written yet */
    }
    // Write on first use, and upgrade an untouched legacy copy so existing
    // installs stop being told this directory is disposable. Anything else the
    // user has in there is theirs.
    if (current === '' || current === LEGACY_HOME_README) {
      if (current !== HOME_README) fs.writeFileSync(readme, HOME_README, 'utf8');
    }
    return dir;
  } catch {
    return os.homedir();
  }
}
