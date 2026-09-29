/** Enumerate actual Rust production files while retaining unreferenced source:
 * adding a file must not evade a source guard merely by omitting its mod line. */
import path from 'path';
import { maskRust, production } from './rustHttpSource';
export function productionFiles(files: Map<string, string>): Map<string, string> {
  const ignored = new Set<string>(),
    live = new Set<string>();
  const resolve = (file: string, declaration: string, name: string): string[] => {
    const parent = path.posix.dirname(file);
    const explicit = /#\[path\s*=\s*"([^"]+)"\]/.exec(declaration);
    if (explicit) return [path.posix.join(parent, explicit[1])];
    const base = ['mod.rs', 'lib.rs', 'main.rs'].includes(path.posix.basename(file))
      ? parent
      : file.replace(/\.rs$/, '');
    return [
      path.posix.join(base, name.replace(/^r#/, '') + '.rs'),
      path.posix.join(base, name.replace(/^r#/, ''), 'mod.rs'),
    ];
  };
  for (const [file, raw] of files) {
    const source = production(raw);
    let code = maskRust(source);
    const tests = [
      ...code.matchAll(
        /#\[cfg\(test\)\]\s*(?:#\[path\s*=[^\]]*\]\s*)?(?:pub(?:\([^)]*\))?\s+)?mod\s+([^\s;]+)\s*;/g,
      ),
    ];
    for (const declaration of tests.reverse()) {
      const start = declaration.index!,
        end = start + declaration[0].length;
      for (const candidate of resolve(file, source.slice(start, end), declaration[1]))
        ignored.add(candidate);
      code = code.slice(0, start) + code.slice(start, end).replace(/[^\n]/g, ' ') + code.slice(end);
    }
    for (const declaration of code.matchAll(
      /(?:#\[path\s*=[^\]]*\]\s*)?\b(?:pub(?:\([^)]*\))?\s+)?mod\s+([^\s;]+)\s*;/g,
    )) {
      const start = declaration.index!,
        end = start + declaration[0].length;
      for (const candidate of resolve(file, source.slice(start, end), declaration[1]))
        live.add(candidate);
    }
  }
  return new Map(
    [...files]
      .filter(([file]) => !ignored.has(file) || live.has(file))
      .map(([file, source]) => [file, production(source)]),
  );
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
