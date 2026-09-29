/** Structural Rust source scanner for the HTTP contract tests. Strings and
 * comments cannot manufacture registrations, calls, braces, or guard evidence. */
function literalEnd(source: string, start: number): number | undefined {
  const raw =
    source[start] === 'r' || source[start] === 'b'
      ? source.slice(start).match(/^(?:b)?r(#+)?"/)
      : null;
  if (raw) {
    const close = '"' + (raw[1] || '');
    const end = source.indexOf(close, start + raw[0].length);
    if (end < 0) throw new Error('unterminated Rust raw string');
    return end + close.length;
  }
  if (source[start] === '"') {
    let end = start + 1;
    while (end < source.length) {
      if (source[end] === '\\') end += 2;
      else if (source[end++] === '"') return end;
    }
    throw new Error('unterminated Rust string');
  }
  const char =
    source[start] === "'"
      ? source.slice(start).match(/^'(?:\\u\{[0-9a-fA-F]+\}|\\.|[^'\\])'/)
      : null;
  return char ? start + char[0].length : undefined;
}
const masks = [new Map<string, string>(), new Map<string, string>()];
export function maskRust(source: string, strings = true): string {
  const cache = masks[strings ? 1 : 0];
  const cached = cache.get(source);
  if (cached !== undefined) return cached;
  const chars = source.split('');
  const blank = (start: number, end: number): void => {
    for (let n = start; n < end; n++) if (source[n] !== '\n') chars[n] = ' ';
  };
  for (let i = 0; i < source.length;) {
    if (source.startsWith('//', i)) {
      const end = source.indexOf('\n', i);
      const stop = end < 0 ? source.length : end;
      blank(i, stop);
      i = stop;
      continue;
    }
    if (source.startsWith('/*', i)) {
      const start = i;
      let depth = 1;
      i += 2;
      while (i < source.length && depth) {
        if (source.startsWith('/*', i)) {
          depth++;
          i += 2;
        } else if (source.startsWith('*/', i)) {
          depth--;
          i += 2;
        } else i++;
      }
      if (depth) throw new Error('unterminated Rust comment');
      blank(start, i);
      continue;
    }
    const end = literalEnd(source, i);
    if (end !== undefined) {
      if (strings) blank(i, end);
      i = end;
      continue;
    }
    i++;
  }
  const result = chars.join('');
  cache.set(source, result);
  return result;
}
export function endOf(code: string, open: number, left = '{', right = '}'): number {
  if (code[open] !== left) throw new Error(`expected ${left} at ${open}`);
  let depth = 1;
  for (let i = open + 1; i < code.length; i++) {
    if (code[i] === left) depth++;
    if (code[i] === right) {
      depth--;
      if (!depth) return i + 1;
    }
  }
  throw new Error(`unterminated ${left}`);
}
export function production(source: string): string {
  let code = maskRust(source);
  const re = /#\[cfg\(test\)\]\s*(?:pub\s+)?mod\s+\w+\s*\{/g;
  for (const match of [...code.matchAll(re)].reverse()) {
    const open = code.indexOf('{', match.index);
    const end = endOf(code, open);
    source =
      source.slice(0, match.index) +
      source.slice(match.index, end).replace(/[^\n]/g, ' ') +
      source.slice(end);
    code = maskRust(source);
  }
  return source;
}
export function body(source: string, symbol: string): string {
  const code = maskRust(source);
  const matches = [
    ...code.matchAll(new RegExp('\\bfn\\s+' + symbol + '\\s*(?:<[^;{}]*>)?\\s*\\(', 'g')),
  ];
  if (matches.length !== 1)
    throw new Error(`expected one definition of ${symbol}, found ${matches.length}`);
  const start = code.indexOf('{', matches[0].index);
  const end = endOf(code, start);
  return source.slice(start + 1, end - 1);
}
export function compact(source: string): string {
  source = maskRust(source, false);
  let result = '';
  for (let i = 0; i < source.length;) {
    const end = literalEnd(source, i);
    if (end !== undefined) {
      result += source.slice(i, end);
      i = end;
    } else {
      if (!/\s/.test(source[i])) result += source[i];
      i++;
    }
  }
  return result;
}
export interface Site {
  file: string;
  server: string;
  pattern: string;
  verbs: Record<string, string>;
  expression: string;
}
export function sites(source: string, file: string, server: string): Site[] {
  const code = maskRust(source);
  const out: Site[] = [];
  if (/\.(?:route_service|nest|fallback|fallback_service)\s*\(/.test(code))
    throw new Error(`${file}: unclassified router mounting form`);
  const publicMacro = /macro_rules!\s+public\s*\{/.exec(code);
  const macroStart = publicMacro ? code.indexOf('{', publicMacro.index) : -1;
  const macroEnd = macroStart < 0 ? -1 : endOf(code, macroStart);
  if (macroStart >= 0) {
    const template = compact(source.slice(macroStart, macroEnd));
    if (
      !template.includes(
        'router=router.route($route,get(||async{asset(include_bytes!(concat!("../../assets/web/",$file)),$mime,"public, max-age=86400",)}),);',
      )
    ) {
      throw new Error('public asset macro changed; review its complete stateless body');
    }
    if ([...maskRust(source.slice(macroStart, macroEnd)).matchAll(/\.route\s*\(/g)].length !== 1)
      throw new Error('public macro added an unclassified route');
  }
  for (const match of code.matchAll(/\.(route|nest_service)\s*\(/g)) {
    if (match.index! > macroStart && match.index! < macroEnd) continue;
    const open = code.indexOf('(', match.index);
    const end = endOf(code, open, '(', ')');
    const expression = source.slice(open + 1, end - 1);
    const literal = /^\s*"([^"\n]+)"\s*,/.exec(expression);
    if (!literal) throw new Error(`${file}: dynamic registration not classified`);
    const handler = expression.slice(literal[0].length);
    const verbs: Record<string, string> = {};
    if (match[1] === 'nest_service') verbs.SERVICE = handler.trim();
    else {
      const handlerCode = maskRust(handler);
      let previousEnd = 0;
      for (const verb of handlerCode.matchAll(
        /\b(get|post|put|delete|patch|head|options|trace|connect|any)\s*\(/g,
      )) {
        if (verb.index! < previousEnd) continue;
        const between = handlerCode.slice(previousEnd, verb.index);
        if (!(previousEnd === 0 ? /^\s*(?:\w+::)*$/.test(between) : /^\s*\.\s*$/.test(between)))
          throw new Error(`${file}: unparsed method-router wrapper`);
        if (verb[1].toUpperCase() in verbs) throw new Error(`${file}: duplicate HTTP verb binding`);
        const start = handler.indexOf('(', verb.index) + 1;
        previousEnd = endOf(handlerCode, start - 1, '(', ')');
        const named = /^\s*((?:\w+::)*\w+)\s*\)/.exec(handler.slice(start));
        verbs[verb[1].toUpperCase()] = named ? named[1] : 'inline';
      }
      if (!/^[\s,]*$/.test(handlerCode.slice(previousEnd)))
        throw new Error(`${file}: unparsed method-router suffix`);
    }
    if (!Object.keys(verbs).length)
      throw new Error(`${file}: unparsed method router ${literal[1]}`);
    out.push({ file, server, pattern: literal[1], verbs, expression });
  }
  for (const match of code.matchAll(/\bpublic!\s*\(/g)) {
    if (macroStart < 0) throw new Error('public macro invoked without reviewed definition');
    const open = code.indexOf('(', match.index);
    const end = endOf(code, open, '(', ')');
    const expression = source.slice(open + 1, end - 1);
    const literal = /^\s*"([^"]+)"/.exec(expression);
    if (!literal) throw new Error('dynamic public macro invocation');
    out.push({ file, server, pattern: literal[1], verbs: { GET: 'public!' }, expression });
  }
  return out;
}
