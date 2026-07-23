/**
 * File-path detection for freeform LLM text (assistant prose, command output),
 * so file mentions in chat get the same clickable FileLink affordance as tool
 * calls. Pure functions — the markdown renderer consumes these.
 *
 * Precision beats recall here: a false positive turns prose into a dead link,
 * so the rules are deliberately conservative and split by context:
 *
 *  - `detectFilePath` (inline-code spans, `like/this.ts`): the whole span must
 *    be one path. Single-segment filenames are allowed only for well-known
 *    code extensions (`package.json` yes, `example.com` no).
 *  - `linkifyText` (bare prose): only path-shaped tokens containing a
 *    separator (absolute, ./-relative, or multi-segment with an extension) —
 *    bare words and dotted identifiers (`notifications.post`) never match.
 *
 * A trailing `:12` / `:12:5` (file:line[:col]) is kept in the display text but
 * stripped from the openable path.
 */

export interface DetectedPath {
  /** The openable path (line suffix stripped). */
  path: string;
  /** 1-based line from a `:line[:col]` suffix, when present. */
  line?: number;
  /** What to render — the original token, line suffix included. */
  display: string;
}

/** Extensions that make a bare single-segment filename path-like. Chosen to
 *  exclude TLDs (`com`, `org`, `net`, `ai`, …) so domains never linkify. */
const SINGLE_SEGMENT_EXTS = new Set([
  'ts',
  'tsx',
  'js',
  'jsx',
  'mjs',
  'cjs',
  'json',
  'jsonc',
  'yaml',
  'yml',
  'toml',
  'ini',
  'md',
  'markdown',
  'txt',
  'log',
  'lock',
  'rs',
  'go',
  'py',
  'rb',
  'java',
  'kt',
  'swift',
  'c',
  'h',
  'cc',
  'cpp',
  'hpp',
  'cs',
  'php',
  'ex',
  'exs',
  'erl',
  'hs',
  'lua',
  'zig',
  'css',
  'scss',
  'less',
  'html',
  'htm',
  'svg',
  'xml',
  'sql',
  'sh',
  'bash',
  'zsh',
  'fish',
  'ps1',
  'bat',
  'cmd',
  'gradle',
  'proto',
  'graphql',
  'tf',
  'nix',
  'env',
  'conf',
  'cfg',
]);

/** Extensionless files worth linking by name. */
const BARE_BASENAMES = new Set([
  'Makefile',
  'makefile',
  'Dockerfile',
  'Justfile',
  'justfile',
  'Rakefile',
  'Gemfile',
  'Procfile',
  'LICENSE',
  'CHANGELOG',
  'Vagrantfile',
]);

/** First path segments that are really domains — reject `github.com/x/y.ts`. */
const DOMAINISH_FIRST_SEGMENT =
  /^[\w-]+\.(com|org|net|io|dev|ai|co|app|sh|gg|xyz|me|us|uk|ca|de|fr|edu|gov)$/i;

const LINE_SUFFIX = /:(\d{1,6})(?::\d{1,6})?$/;
/** Extension: dot + letter-led alnum run (rejects version-ish `v1.2`). */
const EXT = /\.([a-zA-Z][a-zA-Z0-9]{0,7})$/;

function isAbsolute(p: string): boolean {
  return p.startsWith('/') || p.startsWith('\\\\') || /^[a-zA-Z]:[\\/]/.test(p);
}

function basenameOf(p: string): string {
  const parts = p.replace(/\\/g, '/').split('/');
  return parts[parts.length - 1] ?? p;
}

/** Basename qualifies as file-like: known bare name, or has SOME extension
 *  (letter-led). Used for multi-segment paths, where the separators already
 *  did most of the disambiguation. */
function fileLikeBasename(base: string): boolean {
  return BARE_BASENAMES.has(base) || EXT.test(base);
}

/**
 * Classify a whole token (no internal whitespace) as a file path, or null.
 * `context` picks the strictness for single-segment names: 'code' allows
 * `package.json`-style bare filenames, 'prose' requires a separator.
 */
export function detectFilePath(
  raw: string,
  context: 'code' | 'prose' = 'code',
): DetectedPath | null {
  const token = raw.trim();
  if (!token || token.length > 512) return null;
  if (/\s/.test(token)) return null; // multi-word span — a command, not a path
  if (token.includes('://') || token.startsWith('www.')) return null; // URL
  if (token.startsWith('-')) return null; // CLI flag
  if (token.includes('${') || token.includes('*')) return null; // template/glob

  const display = token;
  let path = token;
  let line: number | undefined;
  const lineMatch = LINE_SUFFIX.exec(path);
  if (lineMatch) {
    line = parseInt(lineMatch[1], 10);
    path = path.slice(0, lineMatch.index);
  }
  if (!path || path.endsWith('/') || path.endsWith('\\')) return null; // directory-ish
  if (path.includes(':') && !/^[a-zA-Z]:[\\/]/.test(path)) return null; // scheme/rest of colon uses

  const base = basenameOf(path);
  const hasSep = /[\\/]/.test(path);

  if (isAbsolute(path)) {
    if (!hasSep || !fileLikeBasename(base)) return null;
    return { path, line, display };
  }
  if (path.startsWith('./') || path.startsWith('../')) {
    return fileLikeBasename(base) ? { path, line, display } : null;
  }
  if (hasSep) {
    // Multi-segment relative (src/main/foo.ts). Must start word-like, not be a
    // domain, and end in a file-like basename.
    if (!/^[\w.@-]/.test(path)) return null;
    const first = path.replace(/\\/g, '/').split('/')[0];
    if (DOMAINISH_FIRST_SEGMENT.test(first)) return null;
    return fileLikeBasename(base) ? { path, line, display } : null;
  }
  // Single segment: only in code spans, only for clearly code-ish files.
  if (context === 'prose') return null;
  if (BARE_BASENAMES.has(path)) return { path, line, display };
  const ext = EXT.exec(path)?.[1]?.toLowerCase();
  if (ext && SINGLE_SEGMENT_EXTS.has(ext) && path.length > ext.length + 1) {
    return { path, line, display };
  }
  return null;
}

/** Punctuation that commonly trails a path in prose ("see src/a.ts, then…"). */
const TRAILING_PUNCT = /[.,;!?)\]}'"`]+$/;
const LEADING_PUNCT = /^[(\['"`{]+/;

/**
 * Split prose into literal strings and detected paths, preserving every
 * character (leading/trailing punctuation stays as text around the link).
 */
export function linkifyText(text: string): Array<string | DetectedPath> {
  // Perf guards for the streaming hot path (the in-flight turn re-parses per
  // content tick): no separator anywhere means no candidates — bail in O(n) —
  // and pathological single-run inputs skip scanning entirely.
  if (text.length > 50_000 || !/[\\/]/.test(text)) return [text];
  const out: Array<string | DetectedPath> = [];
  let last = 0;
  // Candidate tokens: any non-whitespace run containing a path separator or an
  // absolute/dot-relative prefix. detectFilePath does the real vetting. The
  // prefix deliberately excludes separators ([^\s\\/]* rather than \S*) so a
  // failed attempt can't backtrack through them.
  const candidate = /[^\s\\/]*[\\/]\S+/g;
  let m: RegExpExecArray | null;
  while ((m = candidate.exec(text)) !== null) {
    let token = m[0];
    let start = m.index;
    const lead = LEADING_PUNCT.exec(token)?.[0] ?? '';
    token = token.slice(lead.length);
    start += lead.length;
    // Trailing punctuation is clause syntax, not path — but keep a `:12` line
    // suffix intact by only trimming AFTER the vetting fails, char by char.
    let trail = '';
    let hit = detectFilePath(token, 'prose');
    while (!hit && TRAILING_PUNCT.test(token)) {
      trail = token.slice(-1) + trail;
      token = token.slice(0, -1);
      hit = detectFilePath(token, 'prose');
    }
    if (!hit) continue;
    if (start > last) out.push(text.slice(last, start));
    out.push(hit);
    last = start + token.length;
  }
  if (last < text.length) out.push(text.slice(last));
  return out.length > 0 ? out : [text];
}
