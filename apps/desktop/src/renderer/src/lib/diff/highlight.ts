/**
 * Lazy shiki-based syntax highlighter for the diff viewer.
 *
 * Built for speed on big diffs:
 *  - JS regex engine (no WASM download/compile at startup)
 *  - grammars load on demand per file extension, then stay cached
 *  - callers tokenize per-hunk virtual documents, so cost scales with the
 *    diff, not the file
 *
 * Tokens carry only foreground colors; backgrounds (add/del tints, emphasis)
 * stay ours so the viewer matches the app theme.
 */

import {
  createHighlighterCore,
  type HighlighterCore,
  type LanguageRegistration,
  type MaybeGetter,
} from '@shikijs/core';
import { createJavaScriptRegexEngine } from '@shikijs/engine-javascript';

export interface TokenSpan {
  text: string;
  color?: string;
}

/** A line longer than this renders unhighlighted — tokenizing minified blobs
 * is where TextMate grammars go to die. */
export const MAX_HIGHLIGHT_LINE_LENGTH = 1000;

type LangLoader = () => Promise<MaybeGetter<LanguageRegistration[]>>;

// Extension (or exact basename) → shiki grammar. Each grammar module ships
// with its embedded dependencies, so one import per entry is enough.
const LANG_BY_EXT: Record<string, string> = {
  ts: 'typescript',
  mts: 'typescript',
  cts: 'typescript',
  tsx: 'tsx',
  js: 'javascript',
  mjs: 'javascript',
  cjs: 'javascript',
  jsx: 'jsx',
  json: 'json',
  jsonc: 'jsonc',
  rs: 'rust',
  go: 'go',
  py: 'python',
  rb: 'ruby',
  php: 'php',
  java: 'java',
  kt: 'kotlin',
  swift: 'swift',
  c: 'c',
  h: 'c',
  cc: 'cpp',
  cpp: 'cpp',
  hpp: 'cpp',
  cs: 'csharp',
  css: 'css',
  scss: 'scss',
  less: 'less',
  html: 'html',
  htm: 'html',
  vue: 'vue',
  svelte: 'svelte',
  md: 'markdown',
  markdown: 'markdown',
  yml: 'yaml',
  yaml: 'yaml',
  toml: 'toml',
  xml: 'xml',
  svg: 'xml',
  sql: 'sql',
  sh: 'shellscript',
  bash: 'shellscript',
  zsh: 'shellscript',
  ps1: 'powershell',
  psm1: 'powershell',
  lua: 'lua',
  zig: 'zig',
  dart: 'dart',
  ex: 'elixir',
  exs: 'elixir',
  hs: 'haskell',
  graphql: 'graphql',
  gql: 'graphql',
  prisma: 'prisma',
  tf: 'terraform',
  hcl: 'hcl',
  ini: 'ini',
  proto: 'proto',
  dockerfile: 'docker',
};

const LANG_LOADERS: Record<string, LangLoader> = {
  typescript: () => import('@shikijs/langs/typescript').then((m) => m.default),
  tsx: () => import('@shikijs/langs/tsx').then((m) => m.default),
  javascript: () => import('@shikijs/langs/javascript').then((m) => m.default),
  jsx: () => import('@shikijs/langs/jsx').then((m) => m.default),
  json: () => import('@shikijs/langs/json').then((m) => m.default),
  jsonc: () => import('@shikijs/langs/jsonc').then((m) => m.default),
  rust: () => import('@shikijs/langs/rust').then((m) => m.default),
  go: () => import('@shikijs/langs/go').then((m) => m.default),
  python: () => import('@shikijs/langs/python').then((m) => m.default),
  ruby: () => import('@shikijs/langs/ruby').then((m) => m.default),
  php: () => import('@shikijs/langs/php').then((m) => m.default),
  java: () => import('@shikijs/langs/java').then((m) => m.default),
  kotlin: () => import('@shikijs/langs/kotlin').then((m) => m.default),
  swift: () => import('@shikijs/langs/swift').then((m) => m.default),
  c: () => import('@shikijs/langs/c').then((m) => m.default),
  cpp: () => import('@shikijs/langs/cpp').then((m) => m.default),
  csharp: () => import('@shikijs/langs/csharp').then((m) => m.default),
  css: () => import('@shikijs/langs/css').then((m) => m.default),
  scss: () => import('@shikijs/langs/scss').then((m) => m.default),
  less: () => import('@shikijs/langs/less').then((m) => m.default),
  html: () => import('@shikijs/langs/html').then((m) => m.default),
  vue: () => import('@shikijs/langs/vue').then((m) => m.default),
  svelte: () => import('@shikijs/langs/svelte').then((m) => m.default),
  markdown: () => import('@shikijs/langs/markdown').then((m) => m.default),
  yaml: () => import('@shikijs/langs/yaml').then((m) => m.default),
  toml: () => import('@shikijs/langs/toml').then((m) => m.default),
  xml: () => import('@shikijs/langs/xml').then((m) => m.default),
  sql: () => import('@shikijs/langs/sql').then((m) => m.default),
  shellscript: () => import('@shikijs/langs/shellscript').then((m) => m.default),
  powershell: () => import('@shikijs/langs/powershell').then((m) => m.default),
  lua: () => import('@shikijs/langs/lua').then((m) => m.default),
  zig: () => import('@shikijs/langs/zig').then((m) => m.default),
  dart: () => import('@shikijs/langs/dart').then((m) => m.default),
  elixir: () => import('@shikijs/langs/elixir').then((m) => m.default),
  haskell: () => import('@shikijs/langs/haskell').then((m) => m.default),
  graphql: () => import('@shikijs/langs/graphql').then((m) => m.default),
  prisma: () => import('@shikijs/langs/prisma').then((m) => m.default),
  terraform: () => import('@shikijs/langs/terraform').then((m) => m.default),
  hcl: () => import('@shikijs/langs/hcl').then((m) => m.default),
  ini: () => import('@shikijs/langs/ini').then((m) => m.default),
  proto: () => import('@shikijs/langs/proto').then((m) => m.default),
  docker: () => import('@shikijs/langs/docker').then((m) => m.default),
};

/** Grammar id for a path, or null when we have nothing for it. */
export function langForPath(path: string): string | null {
  const base = path.split('/').pop()?.toLowerCase() ?? '';
  if (base === 'dockerfile') return 'docker';
  const ext = base.includes('.') ? base.split('.').pop()! : base;
  return LANG_BY_EXT[ext] ?? null;
}

const DARK_THEME = 'github-dark-default';
const LIGHT_THEME = 'github-light-default';

/** True when the app background resolves to a light color. */
function appIsLight(): boolean {
  if (typeof document === 'undefined') return false;
  const probe = document.createElement('span');
  probe.style.color = 'var(--wks-claude-bg, #111)';
  document.body.appendChild(probe);
  const rgb = getComputedStyle(probe).color.match(/\d+/g)?.map(Number) ?? [17, 17, 17];
  probe.remove();
  const [r, g, b] = rgb;
  return 0.299 * r + 0.587 * g + 0.114 * b > 140;
}

let highlighterPromise: Promise<HighlighterCore> | null = null;
let themeName = DARK_THEME;
const loadedLangs = new Set<string>();
const langPromises = new Map<string, Promise<void>>();

function getHighlighter(): Promise<HighlighterCore> {
  if (!highlighterPromise) {
    themeName = appIsLight() ? LIGHT_THEME : DARK_THEME;
    highlighterPromise = createHighlighterCore({
      themes: [
        themeName === LIGHT_THEME
          ? import('@shikijs/themes/github-light-default').then((m) => m.default)
          : import('@shikijs/themes/github-dark-default').then((m) => m.default),
      ],
      langs: [],
      engine: createJavaScriptRegexEngine({ forgiving: true }),
    });
    // Kick off the cpp grammar (637 kB) during an idle frame so the parse cost
    // doesn't block the first C++ diff render.
    highlighterPromise.then(() => {
      const ric = typeof requestIdleCallback !== 'undefined' ? requestIdleCallback : (cb: () => void) => setTimeout(cb, 200);
      ric(() => { ensureLang('cpp').catch(() => {}); });
    });
  }
  return highlighterPromise;
}

async function ensureLang(lang: string): Promise<boolean> {
  if (loadedLangs.has(lang)) return true;
  const loader = LANG_LOADERS[lang];
  if (!loader) return false;
  let pending = langPromises.get(lang);
  if (!pending) {
    pending = (async () => {
      const hl = await getHighlighter();
      await hl.loadLanguage(await loader());
      loadedLangs.add(lang);
    })();
    langPromises.set(lang, pending);
  }
  try {
    await pending;
    return true;
  } catch {
    // A grammar the JS engine can't run — remember the failure and fall back
    // to plain text instead of retrying on every hunk.
    langPromises.set(lang, Promise.resolve());
    return loadedLangs.has(lang);
  }
}

/**
 * Tokenize a multi-line snippet, one TokenSpan[] per input line. Returns null
 * when the language is unknown/unloadable — caller renders plain text.
 */
export async function tokenize(code: string, lang: string): Promise<TokenSpan[][] | null> {
  if (!(await ensureLang(lang))) return null;
  const hl = await getHighlighter();
  try {
    const lines = hl.codeToTokensBase(code, { lang: lang as never, theme: themeName as never });
    return lines.map((line) => line.map((t) => ({ text: t.content, color: t.color })));
  } catch {
    return null;
  }
}
