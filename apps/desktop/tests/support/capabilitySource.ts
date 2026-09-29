import fs from 'fs';
import path from 'path';
import { endOf, maskRust } from './rustHttpSource';
import { argumentsAt, productionFiles } from './rustProductionSource';
export function rustSources(root: string): Map<string, string> {
  const files = new Map<string, string>();
  function walk(directory: string): void {
    for (const entry of fs.readdirSync(path.join(root, directory), { withFileTypes: true })) {
      const name = `${directory}/${entry.name}`;
      if (entry.isSymbolicLink()) throw Error(`unaccounted source symlink ${name}`);
      if (entry.isDirectory()) walk(name);
      else if (name.endsWith('.rs'))
        files.set(name, fs.readFileSync(path.join(root, name), 'utf8'));
    }
  }
  walk('services/hub-rs/src');
  return productionFiles(files);
}
export interface Registration {
  file: string;
  offset: number;
  method: string;
  handler: string;
}
export function registrations(files: Map<string, string>): Registration[] {
  const result: Registration[] = [];
  for (const [file, source] of files) {
    const code = maskRust(source);
    for (const call of code.matchAll(/\.handler\s*\(/g)) {
      const args = argumentsAt(source, code.indexOf('(', call.index));
      const [expression] = args;
      let names: string[];
      if (/^"[\w.]+"$/.test(expression)) names = [JSON.parse(expression)];
      else {
        if (!/^[a-zA-Z_]\w*$/.test(expression))
          throw Error(`unresolved registration ${file}: ${expression}`);
        const loops = [
          ...code.matchAll(new RegExp('\\bfor\\s+' + expression + '\\s+in\\s*\\[', 'g')),
        ].filter((loop) => {
          const open = code.indexOf('[', loop.index),
            end = endOf(code, open, '[', ']');
          const block = code.indexOf('{', end);
          return block < call.index! && endOf(code, block) > call.index!;
        });
        if (loops.length !== 1)
          throw Error(`registration needs one literal loop ${file}: ${expression}`);
        const open = code.indexOf('[', loops[0].index),
          end = endOf(code, open, '[', ']');
        const block = code.indexOf('{', end);
        if (code.slice(end, block).trim() !== '') throw Error(`dynamic loop expression ${file}`);
        if (
          new RegExp('\\blet\\s+(?:mut\\s+)?' + expression + '\\b').test(
            code.slice(block + 1, call.index),
          )
        )
          throw Error(`rebound registration name ${file}`);
        names = JSON.parse(source.slice(open, end).replace(/,\s*]$/, ']'));
        if (
          !names.length ||
          names.some((name) => typeof name !== 'string' || !/^[A-Za-z][\w.]*$/.test(name))
        )
          throw Error(`invalid method list ${file}`);
      }
      for (const method of names)
        result.push({ file, offset: call.index!, method, handler: args.slice(1).join(',') });
    }
  }
  if (result.length < 150) throw Error('Rust registration scan population collapsed');
  return result;
}

import ts from 'typescript';
export interface DesktopRegistration {
  file: string;
  method: string;
  handler: string;
  node: ts.CallExpression;
  source: ts.SourceFile;
}
export function desktopRegistrations(
  root: string,
  overrides = new Map<string, string>(),
): DesktopRegistration[] {
  const result: DesktopRegistration[] = [];
  const files = new Map<string, string>();
  function walk(directory: string): void {
    for (const entry of fs.readdirSync(path.join(root, directory), { withFileTypes: true })) {
      const file = `${directory}/${entry.name}`;
      if (entry.isSymbolicLink()) throw Error(`unaccounted source symlink ${file}`);
      if (entry.isDirectory()) walk(file);
      else if (file.endsWith('.ts') && !file.endsWith('.test.ts'))
        files.set(file, fs.readFileSync(path.join(root, file), 'utf8'));
    }
  }
  walk('apps/desktop/src/main');
  for (const [file, text] of overrides) files.set(file, text);
  for (const [file, text] of files) {
    if (!text.includes('registerCapability')) continue;
    const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
    const aliases = new Set(['registerCapability']);
    let changed = true;
    while (changed) {
      changed = false;
      const visit = (node: ts.Node): void => {
        if (
          ts.isImportSpecifier(node) &&
          node.propertyName?.text === 'registerCapability' &&
          !aliases.has(node.name.text)
        ) {
          aliases.add(node.name.text);
          changed = true;
        }
        if (
          ts.isVariableDeclaration(node) &&
          ts.isIdentifier(node.name) &&
          node.initializer &&
          !aliases.has(node.name.text)
        ) {
          let refers = false;
          const inspect = (candidate: ts.Node): void => {
            if (ts.isIdentifier(candidate) && aliases.has(candidate.text)) refers = true;
            ts.forEachChild(candidate, inspect);
          };
          inspect(node.initializer);
          if (refers) {
            aliases.add(node.name.text);
            changed = true;
          }
        }
        ts.forEachChild(node, visit);
      };
      visit(source);
    }
    // Resolve only literal constant data from the actual imported source. Unknown
    // expressions fail closed rather than silently dropping dynamic registrations.
    function literal(node: ts.Expression, unit = source, seen = new Set<string>()): unknown {
      if (ts.isAsExpression(node) || ts.isParenthesizedExpression(node))
        return literal(node.expression, unit, seen);
      if (ts.isStringLiteral(node)) return node.text;
      if (ts.isArrayLiteralExpression(node))
        return node.elements.flatMap((element) => {
          if (ts.isSpreadElement(element)) {
            const value = literal(element.expression, unit, seen);
            if (!Array.isArray(value)) throw Error('unresolved registration spread');
            return value;
          }
          return [literal(element, unit, seen)];
        });
      if (ts.isObjectLiteralExpression(node)) {
        const value: Record<string, unknown> = {};
        for (const field of node.properties) {
          if (!ts.isPropertyAssignment(field) || !ts.isIdentifier(field.name))
            throw Error('unresolved registration object');
          value[field.name.text] = literal(field.initializer, unit, seen);
        }
        return value;
      }
      if (ts.isPropertyAccessExpression(node)) {
        const value = literal(node.expression, unit, seen);
        if (value && typeof value === 'object' && !Array.isArray(value))
          return (value as Record<string, unknown>)[node.name.text];
      }
      if (ts.isIdentifier(node)) {
        const key = unit.fileName + ':' + node.text;
        if (seen.has(key)) throw Error('cyclic registration constant');
        const next = new Set([...seen, key]);
        for (const statement of unit.statements) {
          if (
            ts.isVariableStatement(statement) &&
            statement.declarationList.flags & ts.NodeFlags.Const
          ) {
            for (const declaration of statement.declarationList.declarations)
              if (
                ts.isIdentifier(declaration.name) &&
                declaration.name.text === node.text &&
                declaration.initializer
              )
                return literal(declaration.initializer, unit, next);
          }
          if (
            ts.isImportDeclaration(statement) &&
            statement.importClause?.name?.text === node.text &&
            ts.isStringLiteral(statement.moduleSpecifier)
          ) {
            const imported =
              path.posix.normalize(
                path.posix.join(path.posix.dirname(unit.fileName), statement.moduleSpecifier.text),
              ) + '.ts';
            const contents = files.get(imported);
            if (!contents) throw Error('unresolved registration import ' + imported);
            const parsed = ts.createSourceFile(imported, contents, ts.ScriptTarget.Latest, true);
            const exported = parsed.statements.find(ts.isExportAssignment);
            if (exported) return literal(exported.expression, parsed, next);
          }
        }
      }
      throw Error('unresolved registration constant ' + node.getText(unit));
    }
    function names(call: ts.CallExpression): string[] {
      const name = call.arguments[0];
      if (name && ts.isStringLiteral(name)) return [name.text];
      if (name && ts.isIdentifier(name)) {
        for (let parent: ts.Node | undefined = call.parent; parent; parent = parent.parent) {
          if (!ts.isForOfStatement(parent) || !ts.isVariableDeclarationList(parent.initializer))
            continue;
          const declarations = parent.initializer.declarations;
          if (
            !(parent.initializer.flags & ts.NodeFlags.Const) ||
            declarations.length !== 1 ||
            !ts.isIdentifier(declarations[0].name) ||
            declarations[0].name.text !== name.text
          )
            continue;
          const value = literal(parent.expression);
          if (
            Array.isArray(value) &&
            value.length &&
            value.every((item) => typeof item === 'string')
          )
            return value;
        }
      }
      throw Error(`unresolved desktop registration ${file}: ${call.getText(source)}`);
    }
    const visit = (node: ts.Node): void => {
      if (
        ts.isCallExpression(node) &&
        ts.isIdentifier(node.expression) &&
        aliases.has(node.expression.text)
      ) {
        if (!node.arguments[1])
          throw Error(`unresolved desktop registration ${file}: ${node.getText(source)}`);
        for (const method of names(node))
          result.push({
            file,
            method,
            handler: node.arguments[1].getText(source),
            node,
            source,
          });
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
  }
  if (result.length < 60) throw Error('Desktop registration scan population collapsed');
  return result;
}
