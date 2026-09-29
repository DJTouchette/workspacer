import ts from 'typescript';
/** Resolve a declaration within the actual source AST. Comments, quoted source,
 * a sibling method, or a dispatcher naming many methods cannot supply a hop. */
export function declaration(source: ts.SourceFile, name: string): ts.Node {
  const found: ts.Node[] = [];
  function visit(node: ts.Node): void {
    if (
      (ts.isFunctionDeclaration(node) ||
        ts.isMethodDeclaration(node) ||
        ts.isVariableDeclaration(node)) &&
      node.name &&
      ts.isIdentifier(node.name) &&
      node.name.text === name
    )
      found.push(node);
    ts.forEachChild(node, visit);
  }
  visit(source);
  if (found.length !== 1) throw Error(`expected one declaration ${name}, got ${found.length}`);
  return found[0];
}
export function call(
  node: ts.Node,
  source: ts.SourceFile,
  expression: string,
  args?: string[],
): ts.CallExpression {
  const found: ts.CallExpression[] = [];
  function visit(candidate: ts.Node): void {
    if (
      ts.isCallExpression(candidate) &&
      candidate.expression.getText(source) === expression &&
      (!args || args.every((arg, index) => candidate.arguments[index]?.getText(source) === arg))
    )
      found.push(candidate);
    ts.forEachChild(candidate, visit);
  }
  visit(node);
  if (found.length !== 1)
    throw Error(`expected one call ${expression}(${args?.join(',') || ''}), got ${found.length}`);
  return found[0];
}

/** A quoted diagnostic containing guard-shaped text must not supply an edge.
 * Preserve expected literal arguments, but require every code byte of the
 * fragment to occupy code (not a string/comment) in the actual source. */
export function hasRustBearing(source: string, fragment: string): boolean {
  const text = compact(source),
    code = maskRust(text),
    expected = maskRust(fragment);
  for (let at = text.indexOf(fragment); at >= 0; at = text.indexOf(fragment, at + 1)) {
    if ([...expected].every((char, index) => /\s/.test(char) || code[at + index] === char))
      return true;
  }
  return false;
}
import { compact, maskRust } from './rustHttpSource';
