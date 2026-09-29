/** Enumerate actual Rust production files while retaining unreferenced source:
 * adding a file must not evade a source guard merely by omitting its mod line. */
import path from 'path';
import { maskRust, production, endOf } from './rustHttpSource';
export function productionFiles(files: Map<string, string>): Map<string, string> {
  type Edge = { target: string; testOnly: boolean };
  const edges = new Map<string, Edge[]>(),
    cleaned = new Map<string, string>();
  // A cfg is test-only only when it is definitely false with test=false.
  // Unknown feature/platform predicates remain possible production inputs.
  function cfg(expr: string): boolean | undefined {
    expr = expr.trim();
    if (expr === 'test') return false;
    const call = /^(all|any|not)\s*\(([\s\S]*)\)$/.exec(expr);
    if (!call) return undefined;
    const parts: string[] = [];
    let depth = 0,
      start = 0;
    for (let i = 0; i < call[2].length; i++) {
      const c = call[2][i];
      if (c === '(') depth++;
      else if (c === ')') depth--;
      else if (c === ',' && depth === 0) {
        parts.push(call[2].slice(start, i));
        start = i + 1;
      }
    }
    if (call[2].slice(start).trim()) parts.push(call[2].slice(start));
    const values = parts.map(cfg);
    if (call[1] === 'not')
      return values.length === 1 && values[0] !== undefined ? !values[0] : undefined;
    if (call[1] === 'all')
      return values.includes(false) ? false : values.every((v) => v === true) ? true : undefined;
    return values.includes(true) ? true : values.every((v) => v === false) ? false : undefined;
  }
  for (const [file, raw] of files) {
    const code = maskRust(raw),
      links: Edge[] = [],
      hidden: [number, number][] = [];
    const parent = path.posix.dirname(file);
    const ordinary = ['mod.rs', 'lib.rs', 'main.rs'].includes(path.posix.basename(file))
      ? parent
      : file.replace(/\.rs$/, '');
    function modules(
      start: number,
      end: number,
      normalBase: string,
      attributeBase: string,
      inheritedTest: boolean,
    ): void {
      const pattern = /((?:#\[[^\]]*\]\s*)*)(?:pub(?:\([^)]*\))?\s+)?\bmod\s+(r#\w+|\w+)\s*([;{])/g;
      pattern.lastIndex = start;
      for (let match = pattern.exec(code); match && match.index < end; match = pattern.exec(code)) {
        const name = match[2].replace(/^r#/, ''),
          attrs = raw.slice(match.index, match.index + match[1].length);
        const testOnly =
          inheritedTest ||
          [...maskRust(attrs).matchAll(/#\[\s*cfg\s*\(([\s\S]*?)\)\s*\]/g)].some(
            (m) => cfg(m[1]) === false,
          );
        const explicit = /#\[\s*path\s*=\s*("(?:\\.|[^"\\])*")\s*\]/.exec(attrs);
        if (/#\[\s*path\b/.test(attrs) && !explicit)
          throw Error('unresolved Rust module path ' + file);
        const selected = explicit
          ? path.posix.normalize(path.posix.join(attributeBase, JSON.parse(explicit[1])))
          : undefined;
        if (match[3] === ';') {
          const targets = selected
            ? [selected]
            : [
                path.posix.join(normalBase, name + '.rs'),
                path.posix.join(normalBase, name, 'mod.rs'),
              ];
          links.push(...targets.map((target) => ({ target, testOnly })));
        } else {
          const open = pattern.lastIndex - 1,
            stop = endOf(code, open);
          if (testOnly) hidden.push([match.index, stop]);
          const directory = selected || path.posix.join(normalBase, name);
          modules(open + 1, stop - 1, directory, directory, testOnly);
          pattern.lastIndex = stop;
        }
      }
    }
    modules(0, raw.length, ordinary, parent, false);
    edges.set(file, links);
    let text = raw;
    for (const [start, end] of hidden.sort((a, b) => b[0] - a[0]))
      text = text.slice(0, start) + text.slice(start, end).replace(/[^\n]/g, ' ') + text.slice(end);
    cleaned.set(file, production(text));
  }
  // First collect descendants of explicit test-only edges. Then restore every
  // path reachable from a production/unreferenced source through live edges.
  // A plain `mod child` in a test-only parent cannot promote its child to live.
  const candidates = new Set<string>(),
    pending: string[] = [];
  for (const links of edges.values())
    for (const edge of links) if (edge.testOnly) pending.push(edge.target);
  while (pending.length) {
    const file = pending.pop()!;
    if (candidates.has(file)) continue;
    candidates.add(file);
    for (const edge of edges.get(file) || []) pending.push(edge.target);
  }
  const live = new Set<string>(),
    queue = [...files.keys()].filter((file) => !candidates.has(file));
  while (queue.length) {
    const file = queue.pop()!;
    if (live.has(file)) continue;
    live.add(file);
    for (const edge of edges.get(file) || []) if (!edge.testOnly) queue.push(edge.target);
  }
  return new Map([...cleaned].filter(([file]) => !candidates.has(file) || live.has(file)));
}
/** Arguments of a call whose opening parenthesis has already been identified in
 * structurally masked code. Nested expressions cannot truncate the first arg. */
export function argumentsAt(source: string, open: number): string[] {
  const code = maskRust(source),
    stack = ['('],
    args: string[] = [];
  let start = open + 1;
  if (code[open] !== '(') throw Error('call opening parenthesis missing');
  for (let i = start; i < code.length; i++) {
    const c = code[i];
    if ('([{'.includes(c)) stack.push(c);
    else if (')]}'.includes(c)) {
      const expected = { ')': '(', ']': '[', '}': '{' }[c];
      if (stack.pop() !== expected) throw Error('unbalanced Rust call');
      if (!stack.length) {
        args.push(source.slice(start, i).trim());
        return args;
      }
    } else if (c === ',' && stack.length === 1) {
      args.push(source.slice(start, i).trim());
      start = i + 1;
    }
  }
  throw Error('unterminated Rust call');
}
