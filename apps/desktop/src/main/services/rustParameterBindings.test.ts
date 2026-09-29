import { describe, it, expect } from 'vitest';
import path from 'path';
import { rustSources } from '../../../tests/support/capabilitySource';
import { rustDispatcherBindings, callerFields } from '../../../tests/support/rustParameterBindings';
import * as policy from '../../../tests/support/capabilityParameters';
const ROOT = path.resolve(__dirname, '../../../../..');
describe('Rust dispatcher caller parameter extraction', () => {
  it('scans actual service dispatch arms rather than response keys or sibling arms', () => {
    const rows = rustDispatcherBindings(rustSources(ROOT));
    // This ratchet covers Value method-dispatch arms, not serde request structs
    // or arbitrary callee inference. Validation-only method matches can add rows
    // while helper extraction removes inline reads. The Rust AST source guard
    // independently checks every captured Go binding through those helpers.
    expect(rows.length).toBeGreaterThan(50);
    expect([
      ...new Set(
        rows
          .filter((row) => row.method.includes('.') && !policy.classified(row.method))
          .map((row) => row.method),
      ),
    ]).toEqual([]);
    expect({
      rows: rows.length,
      bindings: rows.reduce(
        (n, row) => n + row.fields.filter((field) => !!policy.dangerousKind(field)).length,
        0,
      ),
    }).toEqual({ rows: 68, bindings: 47 });
    for (const [method, field] of [
      ['git.status', 'cwd'],
      ['git.stage', 'path'],
      // fs.write.path now goes through a helper; the AST guard requires its
      // original Go binding independently via go-reference.json.
      ['fs.write', 'contents'],
      ['sessions.load', 'filename'],
      ['claude.profiles.add', 'configDir'],
    ]) {
      expect(
        rows.some((row) => row.method === method && row.fields.includes(field)),
        method + '.' + field,
      ).toBe(true);
    }
    const errors = rows.flatMap((row) =>
      row.fields
        .filter(
          (field) =>
            policy.dangerousKind(field) &&
            policy.classified(row.method) &&
            policy.classifyParam(row.method, field).status === 'unclassified',
        )
        .map((field) => row.method + '.' + field),
    );
    expect(errors).toEqual([]);
  });
  it('tracks caller aliases and pointer reads without accepting quoted or comment decoys', () => {
    expect(
      callerFields(
        'let input=&params;let opts=input;opts.get("shell"); params["cwd"];params.pointer("/nested/env");',
        'params',
      ),
    ).toEqual(['cwd', 'env', 'nested', 'shell']);
    expect(
      callerFields(
        'let text=r#"params["command"]"#; // params.get("env")\njson!({"shell":"x"});',
        'params',
      ),
    ).toEqual([]);
    const source =
      'fn call(method:&str,params:Value){let cwd=params["cwd"];match method{"fs.read"=>{params["path"];},"fs.write"=>{params["path"]; params.get("shell");},_=>{}}}';
    const rows = rustDispatcherBindings(new Map([['fixture.rs', source]]));
    expect(rows.find((row) => row.method === 'fs.read')?.fields).toEqual(['cwd', 'path']);
    expect(rows.find((row) => row.method === 'fs.write')?.fields).toEqual(['cwd', 'path', 'shell']);
  });
  it('discovers new real source files and rejects added executable-shaped caller bindings', () => {
    const files = rustSources(ROOT);
    files.set(
      'services/hub-rs/src/services/new_handler.rs',
      'fn call(method:&str,params:Value){let input=&params;match method{"terminals.create"=>{input.get("env");input.get("launcher");},"new.actor"=>{params["shell"];},_=>{}}}',
    );
    const added = rustDispatcherBindings(files).filter((row) =>
      row.file.endsWith('new_handler.rs'),
    );
    expect(added.find((row) => row.method === 'terminals.create')?.fields).toEqual([
      'env',
      'launcher',
    ]);
    expect(policy.classifyParam('terminals.create', 'env').status).toBe('unclassified');
    expect(policy.suspiciousUnknown('launcher')).toBe(true);
    expect(added.some((row) => !policy.classified(row.method))).toBe(true);
  });
});
