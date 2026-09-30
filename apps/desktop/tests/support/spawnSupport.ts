import fs from 'node:fs';
import path from 'node:path';
import ts from 'typescript';
import { desktopRegistrations } from './capabilitySource';
import { desktopParameterFields } from './parameterBindings';
export interface Side {
  observed: boolean;
  disposition: string;
  reason: string;
  evidenceKind: string;
  evidence: string[];
}
export interface SupportPolicy {
  cases: Array<{ name: string; rust: Side; desktop: Side }>;
  mirroredSupported: string[];
  differences: {
    rustOnly: Record<string, string>;
    desktopOnly: Record<string, string>;
    semantics: Record<string, { rust: string; desktop: string; reason: string }>;
  };
}
const sorted = (items: Iterable<string>) => [...new Set(items)].sort();
const equal = (a: Iterable<string>, b: Iterable<string>) =>
  JSON.stringify(sorted(a)) === JSON.stringify(sorted(b));
export function supportStructure(policy: SupportPolicy): string[] {
  const errors: string[] = [];
  if (
    policy.cases.length < 40 ||
    new Set(policy.cases.map((r) => r.name)).size !== policy.cases.length
  )
    errors.push('support policy population collapsed or duplicated');
  const kinds: Record<string, string> = {
    supported: 'implementation',
    refused: 'refusal',
    'host-derived': 'host-value',
    ignored: 'compatibility-policy',
  };
  for (const row of policy.cases) {
    if (!row.rust.observed && !row.desktop.observed)
      errors.push('stale unobserved support row ' + row.name);
    for (const side of ['rust', 'desktop'] as const) {
      const item = row[side];
      if (
        !kinds[item.disposition] ||
        kinds[item.disposition] !== item.evidenceKind ||
        !item.reason.trim() ||
        !item.evidence.length
      )
        errors.push('unreviewed ' + side + ' disposition ' + row.name);
    }
  }
  const mirrored = policy.cases
    .filter((r) => r.rust.disposition === 'supported' && r.desktop.disposition === 'supported')
    .map((r) => r.name);
  if (
    !equal(mirrored, policy.mirroredSupported) ||
    new Set(policy.mirroredSupported).size !== policy.mirroredSupported.length
  )
    errors.push('mirrored supported intersection drifted');
  for (const [side, expected, reasons] of [
    [
      'rust-only',
      policy.cases.filter((r) => r.rust.observed && !r.desktop.observed).map((r) => r.name),
      policy.differences.rustOnly,
    ],
    [
      'desktop-only',
      policy.cases.filter((r) => r.desktop.observed && !r.rust.observed).map((r) => r.name),
      policy.differences.desktopOnly,
    ],
  ] as const) {
    if (!equal(expected, Object.keys(reasons)) || Object.values(reasons).some((r) => !r.trim()))
      errors.push('stale or missing ' + side + ' support explanation');
  }
  if (
    !equal(
      policy.cases.filter((r) => r.rust.disposition !== r.desktop.disposition).map((r) => r.name),
      Object.keys(policy.differences.semantics),
    )
  )
    errors.push('stale or missing semantic support difference');
  for (const row of policy.cases) {
    const difference = policy.differences.semantics[row.name];
    if (
      difference &&
      (difference.rust !== row.rust.disposition ||
        difference.desktop !== row.desktop.disposition ||
        !difference.reason.trim())
    )
      errors.push('contradictory support difference ' + row.name);
  }
  return errors;
}
export function desktopSpawnRoots(root: string, overrides = new Map<string, string>()): string[] {
  const rows = desktopRegistrations(root).filter((r) => r.method === 'agents.spawn');
  if (rows.length !== 1) throw Error('expected one live desktop spawn registration');
  return sorted([
    ...desktopParameterFields(rows[0].node.arguments[1], rows[0].source, true),
    ...workflowRoots(
      overrides.get('apps/desktop/src/main/services/fleetWorkflowCore.ts') ??
        fs.readFileSync(
          path.join(root, 'apps/desktop/src/main/services/fleetWorkflowCore.ts'),
          'utf8',
        ),
    ),
  ]);
}
function tokens(text: string, name: string): string[] {
  const source = ts.createSourceFile(name, text, ts.ScriptTarget.Latest, true);
  const out: string[] = [];
  function visit(node: ts.Node): void {
    const children = node.getChildren(source);
    if (children.length) children.forEach(visit);
    else if (node.kind !== ts.SyntaxKind.EndOfFileToken) {
      const text = ts.isStringLiteral(node) ? node.text : node.getText(source);
      if (text) out.push(node.kind + ':' + text);
    }
  }
  visit(source);
  return out;
}
export function checkDesktopSupport(
  root: string,
  policy: SupportPolicy,
  observed: string[] | undefined = undefined,
  overrides = new Map<string, string>(),
): string[] {
  observed ??= desktopSpawnRoots(root, overrides);
  const errors = supportStructure(policy),
    expected = policy.cases.filter((r) => r.desktop.observed).map((r) => r.name);
  if (observed.length < 20 || !equal(observed, expected))
    errors.push('desktop support source closure drift');
  const cache = new Map<string, string[]>();
  for (const row of policy.cases) {
    for (const evidence of row.desktop.evidence) {
      const split = evidence.indexOf('|');
      if (split < 1) {
        errors.push('invalid desktop support evidence ' + row.name);
        continue;
      }
      const file = evidence.slice(0, split),
        code = evidence.slice(split + 1);
      if (
        !file.startsWith('apps/desktop/src/main/') ||
        !file.endsWith('.ts') ||
        file.endsWith('.test.ts')
      ) {
        errors.push('nonproduction desktop support evidence ' + row.name);
        continue;
      }
      if (!cache.has(file))
        cache.set(
          file,
          tokens(overrides.get(file) ?? fs.readFileSync(path.join(root, file), 'utf8'), file),
        );
      const actual = cache.get(file)!,
        needle = tokens(code, 'anchor.ts');
      if (
        !needle.length ||
        !actual.some((_, i) => needle.every((part, j) => actual[i + j] === part))
      )
        errors.push('desktop support evidence changed ' + row.name + ': ' + evidence);
    }
  }
  return errors;
}

export function workflowRoots(text: string): string[] {
  const source = ts.createSourceFile('workflow.ts', text, ts.ScriptTarget.Latest, true);
  let found: ts.FunctionDeclaration | undefined;
  function find(n: ts.Node): void {
    if (ts.isFunctionDeclaration(n) && n.name?.text === 'workflowSpawn') found = n;
    ts.forEachChild(n, find);
  }
  find(source);
  if (!found) throw Error('missing workflowSpawn');
  const roots = new Set<string>();
  let alias = false;
  function visit(n: ts.Node): void {
    if (
      (ts.isArrowFunction(n) || ts.isFunctionExpression(n)) &&
      n.parameters.some((x) => ts.isIdentifier(x.name) && x.name.text === 'p')
    )
      return;
    if (ts.isVariableDeclaration(n) && ts.isIdentifier(n.name) && n.name.text === 'p') {
      if (
        !n.initializer ||
        !ts.isObjectLiteralExpression(n.initializer) ||
        n.initializer.properties.length !== 1
      )
        throw Error('unresolved workflow caller alias');
      const spread = n.initializer.properties[0];
      if (!ts.isSpreadAssignment(spread)) throw Error('unresolved workflow caller alias');
      let expression = spread.expression;
      while (ts.isParenthesizedExpression(expression) || ts.isAsExpression(expression))
        expression = expression.expression;
      if (!ts.isIdentifier(expression) || expression.text !== 'raw')
        throw Error('unresolved workflow caller alias');
      alias = true;
    }
    if (
      ts.isPropertyAccessExpression(n) &&
      ts.isIdentifier(n.expression) &&
      n.expression.text === 'p'
    )
      roots.add(n.name.text);
    if (
      ts.isElementAccessExpression(n) &&
      ts.isIdentifier(n.expression) &&
      n.expression.text === 'p'
    ) {
      if (!ts.isStringLiteral(n.argumentExpression))
        throw Error('unresolved workflow parameter key');
      roots.add(n.argumentExpression.text);
    }
    ts.forEachChild(n, visit);
  }
  visit(found);
  if (!alias) throw Error('missing workflow caller alias');
  return sorted(roots);
}
