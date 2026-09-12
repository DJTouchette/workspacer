#!/usr/bin/env node
// Static inventory, deliberately independent of the web object's runtime keys.
// Transport evidence is not proof of deployed registration or behavior.
import ts from 'typescript';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const desktop = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = path.resolve(desktop, '../..');
const file = (p) => path.join(desktop, p);
const names = {
  contract: file('src/renderer/src/types/electron.d.ts'),
  preload: file('src/main/preload.ts'),
  web: file('src/renderer/src/backend/webBackend.ts'),
};
const program = ts.createProgram(Object.values(names), {
  target: ts.ScriptTarget.ESNext,
  module: ts.ModuleKind.ESNext,
  moduleResolution: ts.ModuleResolutionKind.Bundler,
  strictNullChecks: true,
  skipLibCheck: true,
});
const checker = program.getTypeChecker();
function find(node, predicate) {
  if (predicate(node)) return node;
  return ts.forEachChild(node, (child) => find(child, predicate));
}
const contract = find(
  program.getSourceFile(names.contract),
  (n) => ts.isInterfaceDeclaration(n) && n.name.text === 'ElectronAPI',
);
const web = find(
  program.getSourceFile(names.web),
  (n) =>
    ts.isVariableDeclaration(n) &&
    n.name.getText() === 'api' &&
    n.initializer &&
    ts.isObjectLiteralExpression(n.initializer),
).initializer;
const preloadCall = find(
  program.getSourceFile(names.preload),
  (n) =>
    ts.isCallExpression(n) &&
    n.expression.getText() === 'contextBridge.exposeInMainWorld' &&
    n.arguments[0]?.text === 'electronAPI',
);
if (!contract || !web || !preloadCall) throw new Error('API declaration shape changed');

// Ask the checker about spreads, so routingAPI and extracted helpers count too.
function members(object) {
  const result = new Map();
  for (const p of object.properties) {
    if (ts.isSpreadAssignment(p)) {
      if (ts.isCallExpression(p.expression)) {
        const signature = checker.getResolvedSignature(p.expression);
        const declaration = signature?.declaration;
        const returned = declaration?.body?.statements?.find(
          (n) => ts.isReturnStatement(n) && n.expression && ts.isObjectLiteralExpression(n.expression),
        );
        if (returned) {
          for (const [name, node] of members(returned.expression)) result.set(name, node);
          continue;
        }
      }
      for (const symbol of checker.getTypeAtLocation(p.expression).getProperties()) {
        result.set(symbol.name, symbol.valueDeclaration ?? symbol.declarations?.[0] ?? p);
      }
    } else if (p.name) result.set(p.name.getText().replace(/^['"]|['"]$/g, ''), p);
  }
  return result;
}
const webMembers = members(web);
const preloadMembers = members(preloadCall.arguments[1]);
const annotationsPath = file('scripts/web-capability-notes.json');
const notes = fs.existsSync(annotationsPath)
  ? JSON.parse(fs.readFileSync(annotationsPath, 'utf8'))
  : {};
function reference(n) {
  if (!n) return null;
  const source = n.getSourceFile();
  return `${path.relative(root, source.fileName)}:${source.getLineAndCharacterOfPosition(n.getStart()).line + 1}`;
}
const rows = [];
for (const symbol of checker.getTypeAtLocation(contract).getProperties()) {
  const declaration = symbol.valueDeclaration ?? symbol.declarations[0];
  const type = checker.getNonNullableType(checker.getTypeOfSymbolAtLocation(symbol, declaration));
  if (!type.getCallSignatures().length) continue;
  const impl = webMembers.get(symbol.name);
  const transport = [];
  const events = [];
  function calls(n) {
    if (ts.isCallExpression(n) && /(?:^|\.)(call|subscribe)$/.test(n.expression.getText())) {
      const arg = n.arguments[0];
      const target = n.expression.getText().endsWith('subscribe') ? events : transport;
      if (arg && ts.isStringLiteral(arg)) target.push(arg.text);
      else if (
        arg &&
        ts.isCallExpression(arg) &&
        arg.expression.getText() === 'qualify' &&
        ts.isStringLiteral(arg.arguments[1])
      )
        transport.push(arg.arguments[1].text);
    }
    ts.forEachChild(n, calls);
  }
  if (impl) calls(impl);
  const inferred = !impl ? 'absent' : transport.length + events.length ? 'transport wired' : 'needs review';
  const note = notes[symbol.name];
  rows.push({
    method: symbol.name,
    optional: Boolean(symbol.flags & ts.SymbolFlags.Optional),
    status: note?.status ?? inferred,
    detail: note?.detail ?? '',
    transport: [...new Set(transport)],
    events: [...new Set(events)],
    contract: reference(declaration),
    preload: reference(preloadMembers.get(symbol.name)),
    web: reference(impl),
  });
}
rows.sort((a, b) => a.method.localeCompare(b.method));
if (rows.length < 150) throw new Error(`Incomplete inventory: only ${rows.length} methods`);
const staleNotes = Object.keys(notes).filter((name) => !rows.some((r) => r.method === name));
const presenceDrift = rows.filter((r) => Boolean(r.web) === (notes[r.method]?.web === 'absent')).map((r) => r.method);
const summary = {};
for (const row of rows) summary[row.status] = (summary[row.status] ?? 0) + 1;
const report = {
  revision: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(),
  summary,
  staleNotes,
  presenceDrift,
  methods: rows,
};
const escape = (s) => String(s).replaceAll('|', '\\|').replaceAll('\n', ' ');
const link = (ref) =>
  ref ? `[source](../${ref.replace(/:\d+$/, '')}#L${ref.split(':').at(-1)})` : 'absent';
const markdown = [
  '# Web capability inventory',
  '',
  `Source revision: \`${report.revision}\` plus working-tree changes.`,
  '',
  'Generated by `npm run audit:web -- --output ../../docs/web-capability-inventory.md` from apps/desktop.',
  '',
  'Includes inherited and optional ElectronAPI functions, preload bindings, and web object spreads. “Transport wired” means static call evidence only: it does not prove runtime availability, authorization, argument parity, or UI behavior. Reviewed exceptions are in scripts/web-capability-notes.json. Native exclusions are separate from server feature gaps.',
  '',
  ...Object.entries(summary).map(([status, count]) => `- ${status}: ${count}`),
  '',
  '| Method | Optional | Status | Evidence / limitation | Preload | Web |',
  '|---|---|---|---|---|---|',
  ...rows.map(
    (r) =>
      `| ${r.method} | ${r.optional ? 'yes' : 'no'} | ${r.status} | ${escape([r.transport.join(', '), r.events.length ? 'events: ' + r.events.join(', ') : '', r.detail].filter(Boolean).join('; '))} | ${link(r.preload)} | ${link(r.web)} |`,
  ),
  '',
].join('\n');
const args = process.argv.slice(2);
const output = args.includes('--json') ? JSON.stringify(report, null, 2) + '\n' : markdown;
if (args.includes('--output')) fs.writeFileSync(args[args.indexOf('--output') + 1], output);
else process.stdout.write(output);
if (
  args.includes('--check') &&
  (staleNotes.length || presenceDrift.length || rows.some((r) => ['absent', 'needs review'].includes(r.status)))
)
  process.exitCode = 1;
