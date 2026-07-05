/**
 * Templating for library items, resolved at insert time.
 *
 *   {{cwd}} {{sessionId}} {{selection}} {{clipboard}}   — auto context vars
 *   {{?Label}} {{?Label:default}}                       — prompt-for-input (form field)
 *
 * Auto vars are filled from the current context; {{?…}} vars are collected from
 * the user via a small form dialog before the text is used.
 *
 * A {{?…}} token is a typed form field. The type is chosen with a `|type` suffix:
 *
 *   {{?Context}}                         paragraph (multi-line) — the default
 *   {{?Context|area}}                    paragraph (explicit)
 *   {{?Service|text}}                    single-line text
 *   {{?Env|select:dev,staging,prod}}     dropdown (first option is the default)
 *   {{?Verbose|toggle:--verbose,}}       checkbox → injects on/off value
 *
 * A default can be given before the `|` with a colon, exactly like the original
 * syntax: {{?Service:payments-api|text}}, {{?Env:staging|select:dev,staging,prod}}.
 * For a toggle, a default of on/true/1/yes starts it checked.
 */

export interface AutoContext {
  cwd?: string;
  sessionId?: string;
  selection?: string;
  clipboard?: string;
}

export type FieldType = 'text' | 'area' | 'select' | 'toggle';

export interface PromptVar {
  /** The full token text inside the braces, e.g. "?Target:default" — used as the map key. */
  token: string;
  label: string;
  type: FieldType;
  /** The value injected when the user doesn't change the field. */
  default: string;
  /** Choices for a `select` field. */
  options?: string[];
  /** Injected when a `toggle` is checked / unchecked. */
  onValue?: string;
  offValue?: string;
  /** Whether a `toggle` starts checked. */
  checkedByDefault?: boolean;
}

const TOKEN_RE = /\{\{\s*([^}]+?)\s*\}\}/g;

const TRUTHY = /^(on|true|1|yes|checked)$/i;

/**
 * Parse the body of a {{?…}} token (everything after the `?`) into a typed field.
 * Backward compatible: a bare token with no `|type` is a paragraph field, and a
 * `:default` still seeds the field's value.
 */
function parseFieldSpec(rest: string): Omit<PromptVar, 'token'> {
  const pipe = rest.indexOf('|');
  const labelPart = pipe >= 0 ? rest.slice(0, pipe) : rest;
  const typePart = pipe >= 0 ? rest.slice(pipe + 1) : '';

  const lci = labelPart.indexOf(':');
  const label = (lci >= 0 ? labelPart.slice(0, lci) : labelPart).trim() || 'Value';
  const rawDefault = lci >= 0 ? labelPart.slice(lci + 1).trim() : '';

  const tci = typePart.indexOf(':');
  const typeName = (tci >= 0 ? typePart.slice(0, tci) : typePart).trim().toLowerCase();
  const args = tci >= 0 ? typePart.slice(tci + 1) : '';

  if (typeName === 'select' || typeName === 'dropdown' || typeName === 'choice') {
    const options = args
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);
    const def = rawDefault && options.includes(rawDefault) ? rawDefault : (options[0] ?? '');
    return { label, type: 'select', default: def, options };
  }
  if (typeName === 'toggle' || typeName === 'checkbox' || typeName === 'bool') {
    const [on = '', off = ''] = args.split(',');
    const onValue = on.trim();
    const offValue = off.trim();
    const checkedByDefault = TRUTHY.test(rawDefault);
    return {
      label,
      type: 'toggle',
      default: checkedByDefault ? onValue : offValue,
      onValue,
      offValue,
      checkedByDefault,
    };
  }
  // `text` → single line; anything else (incl. bare / `area`) → paragraph.
  const type: FieldType = typeName === 'text' ? 'text' : 'area';
  return { label, type, default: rawDefault };
}

/** Collect the distinct {{?…}} form fields in a template, in first-seen order. */
export function parsePromptVars(text: string): PromptVar[] {
  const seen = new Map<string, PromptVar>();
  let m: RegExpExecArray | null;
  TOKEN_RE.lastIndex = 0;
  while ((m = TOKEN_RE.exec(text)) !== null) {
    const inner = m[1].trim();
    if (!inner.startsWith('?')) continue;
    if (!seen.has(inner)) seen.set(inner, { token: inner, ...parseFieldSpec(inner.slice(1)) });
  }
  return Array.from(seen.values());
}

/** Gather the auto context (clipboard + selection are read here). */
export async function gatherAutoContext(base: {
  cwd?: string;
  sessionId?: string;
}): Promise<AutoContext> {
  let selection = '';
  try {
    selection = window.getSelection?.()?.toString() ?? '';
  } catch {
    /* ignore */
  }
  let clipboard = '';
  try {
    clipboard = (await navigator.clipboard?.readText?.()) ?? '';
  } catch {
    /* permissions/none */
  }
  return { cwd: base.cwd, sessionId: base.sessionId, selection, clipboard };
}

/** Substitute all tokens. `values` is keyed by the inner token (incl. the `?`). */
export function applyTemplate(
  text: string,
  ctx: AutoContext,
  values: Record<string, string> = {},
): string {
  return text.replace(TOKEN_RE, (_full, raw: string) => {
    const inner = raw.trim();
    if (inner.startsWith('?')) {
      if (inner in values) return values[inner];
      // Fall back to the field's declared default if the user wasn't prompted.
      return parseFieldSpec(inner.slice(1)).default;
    }
    switch (inner) {
      case 'cwd':
        return ctx.cwd ?? '';
      case 'sessionId':
        return ctx.sessionId ?? '';
      case 'selection':
        return ctx.selection ?? '';
      case 'clipboard':
        return ctx.clipboard ?? '';
      default:
        return ''; // unknown var → empty
    }
  });
}

/** Frame a skill's body with its title/description so the agent reads it as an
 *  instruction block; prompts insert verbatim. */
export function renderItemText(item: {
  kind: string;
  title: string;
  description?: string;
  body: string;
}): string {
  if (item.kind !== 'skill') return item.body;
  const header = item.description
    ? `# Skill: ${item.title}\n${item.description}\n\n`
    : `# Skill: ${item.title}\n\n`;
  return header + item.body;
}
