/**
 * Host-side rendering for library items of kind 'dispatch' — the templates a
 * Fleet Manager references on spawn (`spawn_agent {template, templateParams}`)
 * so its dispatch boilerplate (delivery mode, reporting contract, diagnostic
 * scaffolds) lives in ONE reviewable file instead of being retyped, drifting,
 * in every manager conversation.
 *
 * Placeholder syntax, shared with the renderer's insert-time templating
 * (renderer lib/libraryTemplate.ts) but resolved here from a params MAP rather
 * than a form dialog:
 *
 *   {{task}}              REQUIRED — rendering without it is a hard error
 *   {{delivery:open a PR}} optional, with an EXPLICIT default after the ':'
 *   {{?task}} {{?d:x}}    the same two, in the renderer's prompt-var spelling
 *                         (the leading '?' is tolerated so a template authored
 *                         for the insert dialog renders here unchanged)
 *   {{cwd}}               auto-filled with the spawn's project directory (the
 *                         caller-named cwd, NOT the worktree carved from it)
 *
 * THE ONE HARD RULE, and the reason this does not reuse the renderer's
 * applyTemplate: placeholders are REQUIRED BY DEFAULT, and rendering with an
 * unfilled required placeholder is an ERROR naming the missing param — never a
 * silent default. applyTemplate falls back to the field's declared default (or
 * empty) when the user wasn't prompted, which is right for an interactive form
 * and exactly wrong here: a rendered template READS FINISHED, so a manager that
 * could render one without filling the task slot would dispatch boilerplate
 * without ever writing the task-specific reasoning only it can write. Optional
 * placeholders exist, but only by the template author's explicit ':default'.
 *
 * Unknown params are refused too: a typo'd param name would otherwise "miss"
 * its placeholder and surface only as a confusing missing-required error (or,
 * worse, silently vanish against an optional one).
 */
import { hasNonBlankText } from './asciiWhitespace';

/** Same token shape as the renderer's libraryTemplate.ts. */
const TOKEN_RE = /\{\{\s*([^}]+?)\s*\}\}/g;

/** Auto context vars filled by the host, not the caller's params. */
const AUTO_VARS = new Set(['cwd']);

interface Placeholder {
  /** The param name (leading '?' stripped, default stripped). */
  name: string;
  /** Present only when the template marked the placeholder optional. */
  defaultValue?: string;
}

/** Parse one token's inner text ("task", "?task", "delivery:open a PR"). */
function parsePlaceholder(inner: string): Placeholder {
  const rest = inner.startsWith('?') ? inner.slice(1) : inner;
  const ci = rest.indexOf(':');
  if (ci < 0) return { name: rest.trim() };
  return { name: rest.slice(0, ci).trim(), defaultValue: rest.slice(ci + 1).trim() };
}

/** The distinct placeholder names a dispatch template declares (auto vars
 *  excluded), keyed by name with the first occurrence winning — so a name used
 *  twice, once with a default and once without, is REQUIRED wherever the
 *  spelling without a default appears (each token resolves independently). */
export function dispatchTemplateParams(text: string): Placeholder[] {
  const seen = new Map<string, Placeholder>();
  let m: RegExpExecArray | null;
  TOKEN_RE.lastIndex = 0;
  while ((m = TOKEN_RE.exec(text)) !== null) {
    const p = parsePlaceholder(m[1].trim());
    if (!p.name || AUTO_VARS.has(p.name)) continue;
    if (!seen.has(p.name)) seen.set(p.name, p);
  }
  return Array.from(seen.values());
}

/**
 * Render a dispatch template with the caller's params. Throws (never defaults)
 * on an unfilled required placeholder, and on a param naming no placeholder.
 *
 * `params` values win over auto vars on a name collision; a value must carry
 * non-blank text to count as filling a REQUIRED placeholder — an empty string
 * is the same dodge as omitting it.
 */
export function renderDispatchTemplate(
  text: string,
  params: Record<string, string> = {},
  ctx: { cwd?: string } = {},
): string {
  const declared = new Set<string>();
  TOKEN_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = TOKEN_RE.exec(text)) !== null) declared.add(parsePlaceholder(m[1].trim()).name);
  const unknown = Object.keys(params).filter((k) => !declared.has(k));
  if (unknown.length) {
    throw new Error(
      `dispatch template: unknown templateParams ${unknown.map((k) => `"${k}"`).join(', ')} — ` +
        `this template's placeholders are: ${[...declared].map((n) => `{{${n}}}`).join(' ') || '(none)'}`,
    );
  }
  return text.replace(TOKEN_RE, (_full, raw: string) => {
    const p = parsePlaceholder(String(raw).trim());
    const supplied = params[p.name];
    if (typeof supplied === 'string' && hasNonBlankText(supplied)) return supplied;
    if (AUTO_VARS.has(p.name) && !(p.name in params)) return ctx.cwd ?? '';
    if (p.defaultValue !== undefined) return p.defaultValue;
    // HARD ERROR, by design — see the module header. Never a silent default.
    throw new Error(
      `dispatch template: required placeholder {{${p.name}}} is unfilled — ` +
        `pass templateParams: {"${p.name}": "..."} with the task-specific text ` +
        `(a template may mark a placeholder optional with {{${p.name}:default}})`,
    );
  });
}
