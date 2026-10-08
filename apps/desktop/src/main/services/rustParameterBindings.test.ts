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
    }).toEqual({ rows: 73, bindings: 47 }); // terminals.shells (no params); sessions.taskOutput / taskStop: id + integer arms, no dangerous field; claude.handoffSummaryBrief: session id only
    for (const [method, field] of [
      ['git.status', 'cwd'],
      ['git.stage', 'path'],
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
  it('classifies conditional-write path and inert text without admitting executable parameters', () => {
    expect(policy.pathParameters['fs.compareWrite']).toBe(policy.pathParameters['fs.write']);
    expect(policy.classifyParam('fs.compareWrite', 'path').status).toBe('path');
    for (const field of ['contents', 'expected', 'force']) {
      expect(policy.classifyParam('fs.compareWrite', field)).toMatchObject({
        status: 'decision',
        kind: 'inert',
      });
    }
    for (const field of ['shell', 'command', 'argv', 'env', 'permissionMode']) {
      expect(policy.classifyParam('fs.compareWrite', field).status).toBe('unclassified');
    }
    // Both writes now delegate parameter extraction to write(); this bounded
    // scanner cannot claim those reads. The independent Rust AST guard traces
    // the helper and checks all four fields against the current surface policy.
    const rows = rustDispatcherBindings(rustSources(ROOT));
    for (const method of ['fs.write', 'fs.compareWrite']) {
      expect(rows.filter((row) => row.method === method)).toEqual([
        { file: 'services/hub-rs/src/services/files.rs', method, fields: [] },
      ]);
    }
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
