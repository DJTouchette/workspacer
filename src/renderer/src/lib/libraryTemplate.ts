/**
 * Templating for library items, resolved at insert time.
 *
 *   {{cwd}} {{sessionId}} {{selection}} {{clipboard}}   — auto context vars
 *   {{?Label}} {{?Label:default}}                       — prompt-for-input
 *
 * Auto vars are filled from the current context; {{?…}} vars are collected from
 * the user via a small dialog before the text is used.
 */

export interface AutoContext {
  cwd?: string;
  sessionId?: string;
  selection?: string;
  clipboard?: string;
}

export interface PromptVar {
  /** The full token text inside the braces, e.g. "?Target:default" — used as the map key. */
  token: string;
  label: string;
  default: string;
}

const TOKEN_RE = /\{\{\s*([^}]+?)\s*\}\}/g;

/** Collect the distinct {{?…}} prompt-for-input vars in a template. */
export function parsePromptVars(text: string): PromptVar[] {
  const seen = new Map<string, PromptVar>();
  let m: RegExpExecArray | null;
  TOKEN_RE.lastIndex = 0;
  while ((m = TOKEN_RE.exec(text)) !== null) {
    const inner = m[1].trim();
    if (!inner.startsWith('?')) continue;
    const rest = inner.slice(1);
    const ci = rest.indexOf(':');
    const label = (ci >= 0 ? rest.slice(0, ci) : rest).trim();
    const def = ci >= 0 ? rest.slice(ci + 1).trim() : '';
    if (!seen.has(inner)) seen.set(inner, { token: inner, label: label || 'Value', default: def });
  }
  return Array.from(seen.values());
}

/** Gather the auto context (clipboard + selection are read here). */
export async function gatherAutoContext(base: { cwd?: string; sessionId?: string }): Promise<AutoContext> {
  let selection = '';
  try { selection = window.getSelection?.()?.toString() ?? ''; } catch { /* ignore */ }
  let clipboard = '';
  try { clipboard = (await navigator.clipboard?.readText?.()) ?? ''; } catch { /* permissions/none */ }
  return { cwd: base.cwd, sessionId: base.sessionId, selection, clipboard };
}

/** Substitute all tokens. `values` is keyed by the inner token (incl. the `?`). */
export function applyTemplate(text: string, ctx: AutoContext, values: Record<string, string> = {}): string {
  return text.replace(TOKEN_RE, (_full, raw: string) => {
    const inner = raw.trim();
    if (inner.startsWith('?')) {
      if (inner in values) return values[inner];
      // Fall back to the declared default if the user wasn't prompted.
      const ci = inner.indexOf(':');
      return ci >= 0 ? inner.slice(ci + 1).trim() : '';
    }
    switch (inner) {
      case 'cwd': return ctx.cwd ?? '';
      case 'sessionId': return ctx.sessionId ?? '';
      case 'selection': return ctx.selection ?? '';
      case 'clipboard': return ctx.clipboard ?? '';
      default: return ''; // unknown var → empty
    }
  });
}

/** Frame a skill's body with its title/description so the agent reads it as an
 *  instruction block; prompts insert verbatim. */
export function renderItemText(item: { kind: string; title: string; description?: string; body: string }): string {
  if (item.kind !== 'skill') return item.body;
  const header = item.description ? `# Skill: ${item.title}\n${item.description}\n\n` : `# Skill: ${item.title}\n\n`;
  return header + item.body;
}
