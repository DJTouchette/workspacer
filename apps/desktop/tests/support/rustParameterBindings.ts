import { endOf, maskRust } from './rustHttpSource';
export interface RustBindings {
  file: string;
  method: string;
  fields: string[];
}
/** The Value-based method dispatcher shape used by the Rust service layer.
 * This scans actual caller-index/get/pointer reads inside each concrete arm,
 * and shared pre-dispatch reads. It deliberately reports its covered source
 * population separately from typed request/helper analysis. */
export function rustDispatcherBindings(files: Map<string, string>): RustBindings[] {
  const result: RustBindings[] = [];
  for (const [file, source] of files) {
    const code = maskRust(source);
    for (const fn of code.matchAll(/\bfn\s+\w+\s*\(/g)) {
      const argsOpen = code.indexOf('(', fn.index),
        argsEnd = endOf(code, argsOpen, '(', ')');
      const args = source.slice(argsOpen + 1, argsEnd - 1);
      if (!/\bmethod\s*:\s*&\s*str\b/.test(args)) continue;
      const input = args.match(/\b(\w+)\s*:\s*&?\s*(?:serde_json::)?Value\b/);
      if (!input) continue;
      const open = code.indexOf('{', argsEnd);
      if (open < 0) throw Error('dispatcher body missing ' + file);
      const finish = endOf(code, open),
        text = source.slice(open + 1, finish - 1),
        masked = maskRust(text);
      for (const dispatch of masked.matchAll(/\bmatch\s+method\s*\{/g)) {
        if (
          [...masked.slice(0, dispatch.index)].reduce(
            (depth, c) => depth + (c === '{' ? 1 : c === '}' ? -1 : 0),
            0,
          ) !== 0
        )
          continue;
        const start = masked.indexOf('{', dispatch.index),
          end = endOf(masked, start);
        const prefix = text.slice(0, dispatch.index),
          common = callerFields(prefix, input[1]);
        const content = text.slice(start + 1, end - 1),
          structure = maskRust(content),
          lexical = maskRust(content, false);
        let at = 0;
        while (at < structure.length) {
          while (at < structure.length && /[\s,]/.test(lexical[at])) at++;
          if (at === structure.length) break;
          const arrow = structure.indexOf('=>', at);
          if (arrow < 0) throw Error('unresolved method match arm ' + file);
          const labels = content.slice(at, arrow),
            names = [...labels.matchAll(/"([A-Za-z][\w.]*)"/g)].map((m) => m[1]);
          let cursor = arrow + 2;
          while (/\s/.test(structure[cursor] || '') && cursor < structure.length) cursor++;
          const bodyStart = cursor;
          let depth = 0;
          for (; cursor < structure.length; cursor++) {
            const c = structure[cursor];
            if ('([{'.includes(c)) depth++;
            else if (')]}'.includes(c)) depth--;
            if (c === ',' && depth === 0) break;
            if (c === '}' && depth === 0) {
              cursor++;
              break;
            }
          }
          const own = callerFields(prefix + '\n' + content.slice(bodyStart, cursor), input[1]);
          for (const method of names)
            result.push({ file, method, fields: [...new Set([...common, ...own])].sort() });
          at = cursor + 1;
        }
      }
    }
  }
  return result;
}
export function callerFields(text: string, input: string): string[] {
  const code = maskRust(text),
    aliases = new Set([input]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const match of code.matchAll(/\blet\s+(?:mut\s+)?(\w+)\s*=\s*&?\s*(\w+)\b/g))
      if (aliases.has(match[2]) && !aliases.has(match[1])) {
        aliases.add(match[1]);
        changed = true;
      }
  }
  const fields = new Set<string>();
  for (const alias of aliases) {
    const expression = new RegExp(
      '\\b' +
        alias +
        '\\s*(?:\\[\\s*"((?:\\\\.|[^"\\\\])*)"\\s*\\]|\\.(?:get|get_mut|pointer)\\s*\\(\\s*"((?:\\\\.|[^"\\\\])*)"\\s*\\))',
      'g',
    );
    for (const match of text.matchAll(expression)) {
      if (code.slice(match.index, match.index! + alias.length) !== alias) continue;
      const field = JSON.parse('"' + (match[1] ?? match[2]) + '"');
      for (const part of field.split('/')) if (part) fields.add(part);
    }
  }
  return [...fields].sort();
}
