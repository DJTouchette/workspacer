import { describe, expect, it } from 'vitest';
import path from 'path';
import fs from 'fs';
import os from 'os';
import ts from 'typescript';
import golden from '../../../tests/fixtures/capability-parameter-vocabulary.json';
import { desktopRegistrations } from '../../../tests/support/capabilitySource';
import { desktopParameterFields } from '../../../tests/support/parameterBindings';
import * as policy from '../../../tests/support/capabilityParameters';
const ROOT = path.resolve(__dirname, '../../../../..');
function scan(overrides = new Map<string, string>()): {
  errors: string[];
  count: number;
  fields: Map<string, string[]>;
} {
  const fields = new Map<string, string[]>(),
    errors: string[] = [];
  let count = 0;
  for (const row of desktopRegistrations(ROOT, overrides)) {
    const names = desktopParameterFields(row.node.arguments[1], row.source);
    fields.set(row.method, names);
    if (!policy.classified(row.method)) errors.push('unclassified method ' + row.method);
    for (const name of names) {
      if (policy.suspiciousUnknown(name))
        errors.push('unreviewed dangerous name ' + row.method + '.' + name);
      if (policy.dangerousKind(name)) {
        count++;
        if (policy.classifyParam(row.method, name).status === 'unclassified')
          errors.push('unclassified parameter ' + row.method + '.' + name);
      }
    }
  }
  return { errors, count, fields };
}
function fields(code: string): string[] {
  const file = ts.createSourceFile(
    'fixture.ts',
    `const handler=(params:unknown)=>{${code}}`,
    ts.ScriptTarget.Latest,
    true,
  );
  const handler = (file.statements[0] as ts.VariableStatement).declarationList.declarations[0]
    .initializer!;
  return desktopParameterFields(handler, file);
}
describe('capability parameter bindings', () => {
  it('classifies all actual desktop caller bindings, with idiom canaries and a count ratchet', () => {
    const result = scan();
    expect(result.errors).toEqual([]);
    for (const method of ['fs.read', 'library.remove', 'library.save', 'search.project'])
      expect(
        result.fields.get(method)?.some((name) => !!policy.dangerousKind(name)),
        method,
      ).toBe(true);
    // Exact count is deliberately pinned after reviewing actual AST discoveries.
    // 82: git.pull cwd, git.discard cwd + path (2026-10-08).
    expect(result.count).toBe(82);
  });
  it('pins vocabulary, stems and closed exceptions independently of the implementation', () => {
    expect(policy.dangerousNames).toEqual(golden.params);
    expect([...policy.stems].sort()).toEqual([...golden.stems].sort());
    expect(policy.inertExceptions).toEqual(golden.inertExceptions);
  });
  it('finds nested aliases/types/destructuring and source keys rather than local rename targets', () => {
    for (const code of [
      'const input=(params??{}) as {cwd?:string;opts?:{env?:Record<string,string>;command?:string}};',
      'const {cwd,opts:{env,command}}=(params??{}) as TerminalOpts;',
      'const input=(params??{}) as TerminalOpts; void input.opts.env; void input.opts.command;',
      'const {cwd,opts}=(params??{}) as {cwd?:string;opts?:{env?:Record<string,string>;command?:string}};',
      'const {cwd,opts}=(params??{}) as TerminalOpts;void opts?.env;void opts.command;',
    ])
      expect(fields(code)).toEqual(expect.arrayContaining(['env', 'command']));
    expect(fields('const {path:p,cwd}=(params??{}) as {path?:string;cwd?:string};')).toEqual([
      'cwd',
      'path',
    ]);
    expect(fields('const x=params; const y=x.opts; void y["command"];')).toEqual([
      'command',
      'opts',
    ]);
    expect(fields('const x={command:"fake"}; void x.command; // params.shell\n')).toEqual([]);
    expect(() => fields('void params[computed()];')).toThrow('computed');
  });
  it('keeps per-parameter decisions independent of method classification and unfamiliar aliases', () => {
    for (const [method, param, wanted] of [
      ['fs.read', 'path', 'path'],
      ['fs.read', 'shell', 'unclassified'],
      ['terminals.create', 'cwd', 'decision'],
      ['terminals.create', 'shell', 'decision'],
      ['terminals.create', 'env', 'unclassified'],
      ['terminals.create', 'command', 'unclassified'],
      ['agents.spawn', 'mcpItemIds', 'decision'],
      ['agents.spawn', 'profileId', 'decision'],
      ['agents.spawn', 'script', 'unclassified'],
      ['fs.append', 'path', 'unclassified'],
      ['totally.unknown', 'cwd', 'unclassified'],
    ])
      expect(policy.classifyParam(method, param).status, method + '.' + param).toBe(wanted);
    for (const code of [
      'const {cwd,shell,env}=(params??{}) as X;',
      'const input=(params??{}) as {cwd:string;shell:string;env:object};void input.env;',
    ])
      expect(
        fields(code).filter(
          (name) => policy.classifyParam('terminals.create', name).status === 'unclassified',
        ),
      ).toContain('env');
  });
  it('flags new names by token shape while preserving intentional harmless names and exact JS spelling', () => {
    for (const name of [
      'entrypoint',
      'exe',
      'launcher',
      'shellPath',
      'execPath',
      'exec_path',
      'binPath',
      'commandLine',
      'argv0',
      'ENTRYPOINT',
      'launchCommand',
      'envVarsExtra',
      'downloadUrl',
      'workDirectory',
    ])
      expect(policy.suspiciousUnknown(name), name).toBe(true);
    for (const name of [
      'path',
      'cwd',
      'shell',
      'command',
      'sessionId',
      'agentId',
      'title',
      'body',
      'level',
      'limit',
      'cols',
      'rows',
      'permissionMode',
      'model',
      'provider',
      'transport',
      'staged',
      'untracked',
      'contents',
      'message',
      'activeTabId',
      'tabs',
      'answers',
      'silent',
    ])
      expect(policy.suspiciousUnknown(name), name).toBe(false);
    for (const [spelling, canonical] of [
      ['env', 'env'],
      ['Env', 'env'],
      ['ENV', 'env'],
      ['configdir', 'configDir'],
      ['ConfigDir', 'configDir'],
      ['BytesB64', 'bytesB64'],
      ['Shell', 'shell'],
      ['ExtraArgs', 'extraArgs'],
    ]) {
      expect(policy.foldName(spelling, Object.keys(policy.dangerousNames))).toBe(canonical);
      expect(policy.dangerousKind(spelling, true)).toBe(policy.dangerousKind(canonical));
    }
    expect(policy.dangerousKind('Env')).toBeUndefined();
    expect(policy.dangerousKind('sessionId', true)).toBeUndefined();
    expect(policy.foldName('ſkipPermissions', ['skipPermissions'])).toBe('skipPermissions');
    expect(policy.foldName('sKipPermissions', ['skipPermissions'])).toBe('skipPermissions');
    expect(policy.foldName('İd', ['id'])).toBeUndefined();
  });
  it('discovers a new module or a newly bound dangerous field in the actual registry, and ratchets both directions', () => {
    const module = 'apps/desktop/src/main/services/newParameterRegistry.ts';
    const unknown = scan(
      new Map([
        [
          module,
          "import {registerCapability} from './hubClient'; registerCapability('new.actor',(params:unknown)=>{const {shell}=(params??{}) as any;});",
        ],
      ]),
    );
    expect(unknown.errors).toContain('unclassified method new.actor');
    expect(unknown.errors).toContain('unclassified parameter new.actor.shell');
    const renamed = scan(
      new Map([
        [
          module,
          "import {registerCapability as add} from './hubClient'; add('terminals.create',(params:unknown)=>{const input=params as {nested:{launcher:string}};void input.nested.launcher;});",
        ],
      ]),
    );
    expect(renamed.errors).toContain('unreviewed dangerous name terminals.create.launcher');
    const added = scan(
      new Map([
        [
          module,
          "import {registerCapability} from './hubClient'; registerCapability('terminals.create',(params:unknown)=>{const {env}=(params??{}) as any;});",
        ],
      ]),
    );
    expect(added.errors).toContain('unclassified parameter terminals.create.env');
    expect(() => policy.ratchet(added.count, 79)).toThrow('ratchet');
    expect(() => policy.ratchet(78, 79)).toThrow('ratchet');
    expect(() => policy.ratchet(79, 79)).not.toThrow();
  });
  it('reads fresh source, treats missing checkout files as errors, and preserves CRLF binding coverage', () => {
    const file = 'apps/desktop/src/main/services/hubCapabilities.ts';
    const source = fs.readFileSync(path.join(ROOT, file), 'utf8');
    const windows = scan(new Map([[file, source.replace(/\r?\n/g, '\r\n')]]));
    expect(windows.errors).toEqual([]);
    expect(windows.count).toBe(82);
    const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'capspec-reader-'));
    try {
      expect(() => desktopRegistrations(temporary)).toThrow();
      fs.mkdirSync(path.join(temporary, 'apps/desktop/src/main'), { recursive: true });
      expect(() => desktopRegistrations(temporary)).toThrow('population');
    } finally {
      fs.rmSync(temporary, { recursive: true, force: true });
    }
    expect(() =>
      desktopRegistrations(
        ROOT,
        new Map([['apps/desktop/src/main/shared/desktopServices.generated.ts', '']]),
      ),
    ).toThrow('import');
  });
  it('does not inherit JavaScript prototype fields or use regex punctuation as a wildcard', () => {
    for (const name of ['constructor', '__proto__', 'toString']) {
      expect(policy.classified(name)).toBe(false);
      expect(policy.dangerousKind(name)).toBeUndefined();
      expect(policy.classifyParam('agents.spawn', name).status).toBe('unclassified');
    }
    expect(policy.foldName('terminalXshell', ['terminal.shell'])).toBeUndefined();
    expect(policy.foldName('TERMINAL.SHELL', ['terminal.shell'])).toBe('terminal.shell');
  });
});
