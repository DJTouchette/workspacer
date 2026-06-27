/**
 * Project-wide text search for the editor pane's search sidebar.
 *
 * Like fileService, this is the app's own fs layer (main is the trusted backend
 * for the renderer-as-editor) and is exposed two ways: `search:project` IPC for
 * the desktop renderer and the `search.project` hub capability for the web/phone
 * client. Both call straight into searchProject below.
 *
 * Implementation shells out to ripgrep (`rg --json`) rather than walking the
 * tree in Node: rg is fast, already gitignore-aware, and gives us line + column
 * for free. We stream its JSONL output, group matches per file, and stop early
 * once the result cap is hit so a giant repo can't flood the renderer.
 */
import { execFile } from 'child_process';
import * as path from 'path';
import * as fs from 'fs';

/**
 * Resolve @vscode/ripgrep's prebuilt binary path.
 *
 * @vscode/ripgrep is ESM-only (and exports `rgPath` via a platform-specific
 * optional dependency), so a static `import` of it breaks our CommonJS main
 * build with ERR_REQUIRE_ESM. We replicate its tiny resolution logic here with
 * CommonJS `require.resolve` instead: find the prebuilt `rg` inside the
 * per-platform package (`@vscode/ripgrep-<platform>-<arch>`). Returns undefined
 * if the optional dependency isn't installed for this platform, in which case we
 * fall back to a PATH lookup below.
 */
function resolveBundledRgPath(): string | undefined {
  const arch = process.env.npm_config_arch || process.arch;
  const binaryName = process.platform === 'win32' ? 'rg.exe' : 'rg';
  const platformPkg = `@vscode/ripgrep-${process.platform}-${arch}`;
  try {
    return require.resolve(`${platformPkg}/bin/${binaryName}`);
  } catch {
    return undefined;
  }
}

/**
 * Absolute path to the ripgrep binary we shell out to.
 *
 * We ship @vscode/ripgrep's prebuilt binary rather than relying on a system
 * `rg`: a GUI-launched Electron app inherits a stripped PATH and usually can't
 * find one, so bare `execFile('rg')` fails with ENOENT (the "search does
 * nothing" bug this replaces).
 *
 * @vscode/ripgrep is now ESM-only and can't be `require()`d from our CommonJS
 * main bundle, so we resolve the platform binary ourselves the same way the
 * module does (its only job): require.resolve the platform-specific subpackage
 * (@vscode/ripgrep-<platform>-<arch>), which lives in optionalDependencies.
 *
 * In a packaged build the dependency lives inside app.asar, which can't be
 * executed — electron-builder unpacks it to app.asar.unpacked (see
 * electron-builder.yml asarUnpack), so rewrite the path accordingly. If the
 * bundled binary is somehow missing, fall back to a PATH lookup so dev still
 * works.
 */
const RG_BIN: string = (() => {
  const bundledRgPath = resolveBundledRgPath();
  if (bundledRgPath) {
    const unpacked = bundledRgPath.replace(/\bapp\.asar\b/, 'app.asar.unpacked');
    try {
      if (fs.existsSync(unpacked)) return unpacked;
    } catch { /* fall through */ }
  }
  return 'rg';
})();

/** Hard cap on total matches returned when the caller doesn't specify one. */
const DEFAULT_MAX_RESULTS = 500;
/** Matching lines are display-only; clip very long lines so the UI stays sane. */
const MAX_TEXT_LEN = 300;
/** Bound rg's runtime and output so a pathological repo can't hang/OOM main. */
const EXEC_TIMEOUT_MS = 15_000;
const EXEC_MAX_BUFFER = 64 * 1024 * 1024;

export interface SearchProjectOpts {
  query: string;
  cwd: string;
  caseSensitive?: boolean;
  wholeWord?: boolean;
  regex?: boolean;
  maxResults?: number;
}

export interface SearchMatch {
  line: number; // 1-based
  column: number; // 1-based
  text: string; // trimmed matching line, clipped to MAX_TEXT_LEN
}
export interface SearchFileResult {
  file: string; // absolute path
  matches: SearchMatch[];
}
export interface SearchProjectResult {
  results: SearchFileResult[];
  truncated: boolean;
}

/** Subset of ripgrep's --json message shape we actually read. */
interface RgMatchMessage {
  type: 'match';
  data: {
    path: { text?: string };
    lines: { text?: string };
    line_number: number;
    submatches: Array<{ start: number }>;
  };
}

export async function searchProject(opts: SearchProjectOpts): Promise<SearchProjectResult> {
  const { query, cwd } = opts;
  if (!query) return { results: [], truncated: false };

  const maxResults = opts.maxResults ?? DEFAULT_MAX_RESULTS;

  // Flags mirror the contract. Default is smart-case; explicit case sensitivity
  // wins. Fixed-string (-F) unless the caller asked for regex. --json carries
  // line/column data, so we don't also need --line-number/--column for parsing,
  // but pass them so plain reuse of the argv stays correct.
  const args = ['--json', '--line-number', '--column'];
  if (opts.caseSensitive) args.push('-s');
  else args.push('--smart-case');
  if (opts.wholeWord) args.push('-w');
  if (!opts.regex) args.push('-F');
  // Pattern then search root. The trailing '.' (relative to `cwd`) is REQUIRED:
  // with no path argument and no controlling TTY (always the case under
  // execFile), ripgrep reads from stdin instead of the directory and returns
  // nothing — the original "search does nothing" bug.
  args.push('--', query, '.');

  const stdout = await new Promise<string>((resolve, reject) => {
    execFile(
      RG_BIN,
      args,
      { cwd, timeout: EXEC_TIMEOUT_MS, maxBuffer: EXEC_MAX_BUFFER, encoding: 'utf8' },
      (err, out) => {
        // rg exits 1 when there were simply no matches — that's success here.
        // Exit >=2 is a real error (bad pattern, unreadable cwd); ENOENT means
        // the bundled binary was lost and the PATH fallback found nothing.
        const code = (err as { code?: number } | null)?.code;
        if (err && code !== 1) {
          reject(new Error(`ripgrep failed (exit ${code}): ${err.message}`));
          return;
        }
        resolve(out);
      },
    );
  });

  // Group matches by file, capping total matches. Once the cap is hit we stop
  // parsing further lines and flag the result as truncated.
  const byFile = new Map<string, SearchFileResult>();
  let total = 0;
  let truncated = false;

  for (const line of stdout.split('\n')) {
    if (!line) continue;

    let msg: { type?: string };
    try { msg = JSON.parse(line); } catch { continue; }
    if (msg.type !== 'match') continue;

    // Cap check AFTER confirming this is a real match: a genuine, dropped match
    // is the only thing that counts as truncation. Checking before the type
    // guard let ripgrep's trailing `end`/`summary` messages flip the flag when
    // the count landed exactly on maxResults (nothing was actually dropped).
    if (total >= maxResults) { truncated = true; break; }

    const data = (msg as RgMatchMessage).data;
    const rel = data.path.text;
    if (!rel) continue;
    // rg reports paths relative to cwd; the contract wants absolute paths.
    const abs = path.resolve(cwd, rel);
    const rawText = data.lines.text ?? '';
    const text = rawText.replace(/\r?\n$/, '').trim().slice(0, MAX_TEXT_LEN);

    let bucket = byFile.get(abs);
    if (!bucket) { bucket = { file: abs, matches: [] }; byFile.set(abs, bucket); }

    // rg emits one 'match' message per matching line, but a line may contain
    // several submatches. Surface each as its own result (column = 1-based byte
    // offset of the submatch start).
    const submatches = data.submatches.length ? data.submatches : [{ start: 0 }];
    for (const sm of submatches) {
      if (total >= maxResults) { truncated = true; break; }
      bucket.matches.push({ line: data.line_number, column: sm.start + 1, text });
      total += 1;
    }
  }

  return { results: [...byFile.values()], truncated };
}
