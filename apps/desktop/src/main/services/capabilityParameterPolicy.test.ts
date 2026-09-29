import { describe, expect, it } from 'vitest';
import path from 'path';
import ts from 'typescript';
import fs from 'fs';
import vocabulary from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
import hostConfig from '../../../../../contracts/host-trusted-config-cases.json';
import * as policy from '../../../tests/support/capabilityParameters';
import { desktopRegistrations, rustSources } from '../../../tests/support/capabilitySource';
import { desktopParameterFields } from '../../../tests/support/parameterBindings';
import { maskRust } from '../../../tests/support/rustHttpSource';
const ROOT = path.resolve(__dirname, '../../../../..');
const KINDS = [
  'path',
  'filename',
  'executable',
  'argv',
  'shell',
  'env',
  'url',
  'port',
  'id',
  'regex',
  'permission',
  'inert',
];
function recordsValid(decisions = policy.parameterDecisions): void {
  const names = [
    ...Object.keys(policy.pathParameters),
    ...Object.keys(policy.methodDecisions),
    ...Object.keys(policy.inertMethods),
  ];
  if (
    new Set(names).size !== names.length ||
    JSON.stringify(names.sort()) !== JSON.stringify([...vocabulary.methods].sort())
  )
    throw Error('method classification overlap or omission');
  for (const method of Object.keys(policy.pathParameters))
    if (!policy.looksPathBearing(method)) throw Error('unclassified path namespace ' + method);
  for (const [method, reason] of [
    ...Object.entries(policy.methodDecisions),
    ...Object.entries(policy.inertMethods),
  ])
    if (!reason.trim()) throw Error('missing classification reason ' + method);
  for (const [method, params] of Object.entries(decisions)) {
    if (
      !Object.hasOwn(policy.pathParameters, method) &&
      !Object.hasOwn(policy.methodDecisions, method)
    )
      throw Error('parameters on unknown or inert method ' + method);
    for (const [name, decision] of Object.entries(params)) {
      if (name === policy.pathParameters[method])
        throw Error('parameter classified twice ' + method + '.' + name);
      if (!KINDS.includes(decision.kind) || !decision.reason.trim())
        throw Error('missing parameter kind/reason ' + method + '.' + name);
      if (method !== 'config.save' && !policy.dangerousKind(name))
        throw Error('parameter outside scanner vocabulary ' + method + '.' + name);
    }
  }
  if (
    JSON.stringify(Object.keys(decisions['config.save']).sort()) !==
    JSON.stringify([...hostConfig.sections, ...hostConfig.paths].sort())
  )
    throw Error('config decisions differ from host-trusted contract');
}
function guards(overrides = new Map<string, string>()): void {
  let direct = 0,
    wrapper = 0,
    claims = 0,
    inert = 0;
  for (const row of desktopRegistrations(ROOT, overrides)) {
    if (policy.inertMethods[row.method] && policy.looksPathBearing(row.method)) {
      if (
        !policy.inertMethods[row.method].includes('no params') ||
        desktopParameterFields(row.node.arguments[1], row.source).length
      )
        throw Error('path-bearing inert method accepts caller values ' + row.method);
      inert++;
    }
    const reason = policy.methodDecisions[row.method] || '';
    const claimed = ['guardGitCwd', 'assertPathAllowed'].find((name) => reason.includes(name));
    if (!policy.pathParameters[row.method] && !claimed) continue;
    let found = false;
    const visit = (node: ts.Node): void => {
      if (
        ts.isCallExpression(node) &&
        ts.isIdentifier(node.expression) &&
        /^(assertPathAllowed|guard[A-Za-z]+)$/.test(node.expression.text) &&
        node.arguments[0] &&
        ts.isStringLiteral(node.arguments[0]) &&
        node.arguments[0].text === row.method
      ) {
        found = true;
        if (node.expression.text === 'assertPathAllowed') direct++;
        else wrapper++;
      }
      ts.forEachChild(node, visit);
    };
    visit(row.node.arguments[1]);
    if (!found) throw Error('missing own-method canonicalization ' + row.method);
    if (claimed) claims++;
  }
  if (!direct || !wrapper || claims < 9 || !inert)
    throw Error('canonicalization witness population collapsed');
}
const HELPERS = [
  'assertPathAllowed',
  'guardGitCwd',
  'guardLibraryFile',
  'assertCommitHash',
  'scrubBypassProfile',
  'scrubProfileBypass',
  'resolveTerminalShell',
  'openExternalUrl',
  'sessionFilePath',
  'resolveWithinSessionsDir',
  'layoutFilePath',
  'claudeProjectDirName',
  'dropHostTrusted',
  'workRoot',
  'resolveInside',
  'resolveSessionFilename',
  'buildSessionMcpConfig',
  'libraryItemRoots',
  'profilesPath',
  'slugSession',
  'assertLibraryItemPath',
  'slugLibrary',
  'anchorGitPathspec',
  'cwdPathspec',
  'containInWorktree',
  'applyLiveEffort',
];
function helperDefinitions(): Set<string> {
  const found = new Set<string>();
  function walk(directory: string): void {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      if (entry.isSymbolicLink()) throw Error('unreviewed source symlink ' + entry.name);
      const file = path.join(directory, entry.name);
      if (entry.isDirectory()) walk(file);
      else if (/\.tsx?$/.test(file) && !file.includes('.test.')) {
        const source = ts.createSourceFile(
          file,
          fs.readFileSync(file, 'utf8'),
          ts.ScriptTarget.Latest,
          true,
        );
        const visit = (node: ts.Node): void => {
          if (
            (ts.isFunctionDeclaration(node) ||
              ts.isMethodDeclaration(node) ||
              ts.isVariableDeclaration(node)) &&
            node.name &&
            ts.isIdentifier(node.name)
          )
            found.add(node.name.text);
          ts.forEachChild(node, visit);
        };
        visit(source);
      }
    }
  }
  walk(path.join(ROOT, 'apps/desktop/src/main'));
  for (const text of rustSources(ROOT).values())
    for (const match of maskRust(text).matchAll(/\b(?:fn|struct|enum|const|static|type)\s+(\w+)/g))
      found.add(match[1]);
  return found;
}
describe('capability parameter policy closure', () => {
  it('classifies every method once, every parameter by kind/reason, and config keys exactly', () =>
    expect(() => recordsValid()).not.toThrow());
  it('checks real own-method path helpers and parameter-free inert namespace exceptions', () =>
    expect(() => guards()).not.toThrow());
  it('retains missing-capability and path-namespace semantics without creating path grants', () => {
    for (const name of [
      'fs.read',
      'search.project',
      'library.save',
      'git.diff',
      'git.status',
      'git.push',
    ]) {
      expect(policy.looksPathBearing(name)).toBe(true);
      expect(policy.missingSpec(name)).toBe(false);
    }
    for (const name of ['fs.append', 'fs.copy', 'search.files', 'library.export', 'git.blame']) {
      expect(policy.looksPathBearing(name)).toBe(true);
      expect(policy.missingSpec(name)).toBe(true);
    }
    for (const name of ['agents.spawn', 'terminals.create', 'config.get', ''])
      expect(policy.looksPathBearing(name)).toBe(false);
    expect(policy.methodDecisions['claude.setModel']).toContain('legacy compatibility spelling');
  });
  it('refuses missing kinds, unbound decisions, unknown fields and lost config keys', () => {
    const mutate = (fn: (copy: typeof policy.parameterDecisions) => void): void => {
      const copy = structuredClone(policy.parameterDecisions);
      fn(copy);
      expect(() => recordsValid(copy)).toThrow();
    };
    mutate((copy) => {
      copy['terminals.create']['env'] = {
        kind: 'unknown',
        reason: 'arbitrary prose pretending an environment has been reviewed',
      };
    });
    mutate((copy) => {
      copy['unknown.method'] = {
        cwd: { kind: 'path', reason: 'arbitrary prose pretending a new method is reviewed' },
      };
    });
    mutate((copy) => {
      copy['terminals.create']['launcher'] = {
        kind: 'executable',
        reason: 'not in the name vocabulary so a scanner would never inspect it',
      };
    });
    mutate((copy) => {
      delete copy['config.save']['terminal.shell'];
    });
    mutate((copy) => {
      copy['providers.checkAll'] = {
        shell: {
          kind: 'executable',
          reason: 'a known inert method cannot launder a newly acting field',
        },
      };
    });
    const file = 'apps/desktop/src/main/services/hubCapabilities.ts';
    const source = fs.readFileSync(path.join(ROOT, file), 'utf8');
    expect(() =>
      guards(new Map([[file, source.replace("guardGitCwd('git.stage', cwd)", 'cwd')]])),
    ).toThrow('git.stage');
  });
  it('keeps named helper claims tied to live production definitions', () => {
    const defined = helperDefinitions(),
      missing: string[] = [];
    let count = 0;
    for (const [method, params] of Object.entries(policy.parameterDecisions))
      for (const [name, decision] of Object.entries(params))
        for (const helper of HELPERS) {
          if (!new RegExp('\\b' + helper + '\\b').test(decision.reason)) continue;
          count++;
          if (!defined.has(helper)) missing.push(method + '.' + name + ': ' + helper);
        }
    expect(count).toBeGreaterThan(0);
    expect(missing).toEqual([]);
  });
});
