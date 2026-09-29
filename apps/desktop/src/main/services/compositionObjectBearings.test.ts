import { describe, expect, it } from 'vitest';
import fs from 'fs';
import path from 'path';
import ts from 'typescript';
import vocabulary from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
import { desktopRegistrations } from '../../../tests/support/capabilitySource';
import { declaration, call, hasRustBearing } from '../../../tests/support/compositionSource';
import { body, compact, endOf, maskRust } from '../../../tests/support/rustHttpSource';
const ROOT = path.resolve(__dirname, '../../../../..');
const CAPS = 'apps/desktop/src/main/services/hubCapabilities.ts';
const REPLAY = 'apps/desktop/src/main/services/timelineReplayService.ts';
const AUTH = 'services/hub-rs/src/auth.rs';
const RUNTIME = 'services/hub-rs/src/runtime.rs';
function verify(overrides = new Map<string, string>(), topics = vocabulary.topics): void {
  const read = (file: string): string =>
    overrides.get(file) ?? fs.readFileSync(path.join(ROOT, file), 'utf8');
  const rows = desktopRegistrations(ROOT, overrides);
  const caps = ts.createSourceFile(CAPS, read(CAPS), ts.ScriptTarget.Latest, true);
  const replay = ts.createSourceFile(REPLAY, read(REPLAY), ts.ScriptTarget.Latest, true);
  for (const method of ['replay.read', 'replay.diff', 'replay.seek']) {
    const row = rows.filter((row) => row.method === method);
    if (row.length !== 1) throw Error(`missing replay registration ${method}`);
    call(row[0].node.arguments[1], row[0].source, 'guardReplaySession', [
      JSON.stringify(method).replaceAll('"', "'"),
      'sessionId',
    ]);
    call(row[0].node.arguments[1], row[0].source, 'timelineReplay.' + method.split('.')[1]);
  }
  const guard = declaration(caps, 'guardReplaySession');
  call(guard, caps, 'timelineReplay.originCwd', ['sessionId']);
  call(guard, caps, 'assertPathAllowed', ['cap', 'origin', 'workspaceRoots()']);
  const readMethod = declaration(replay, 'read');
  call(readMethod, replay, 'this.resolveInside', ['entry', 'p']);
  call(readMethod, replay, 'fs.openSync', ['abs', "'r'"]);
  call(declaration(replay, 'resolveInside'), replay, 'containInWorktree', [
    'entry.dir',
    'entry.scope',
    'path.resolve(entry.dir, rel)',
  ]);
  const contain = declaration(replay, 'containInWorktree');
  call(contain, replay, 'scopedWorktreeRoot', ['dir', 'scope']);
  call(contain, replay, 'canonicalizePath', ['abs']);
  call(contain, replay, 'isWithin', ['canonical', 'root']);
  const condition = contain
    .getChildren(replay)
    .flatMap((node) => (ts.isBlock(node) ? [...node.statements] : []));
  if (
    !condition.some(
      (node) =>
        ts.isIfStatement(node) &&
        node.expression.getText(replay) === '!isWithin(canonical, root)' &&
        ts.isBlock(node.thenStatement) &&
        node.thenStatement.statements.some(ts.isThrowStatement),
    )
  )
    throw Error('replay containment must refuse an escaped canonical path');

  const topic = topics.filter((row) => row.Pattern === 'pty.bytes.*');
  if (
    topic.length !== 1 ||
    topic[0].Disposition !== 'guarded-by-capability' ||
    topic[0].Method !== 'sessions.attachTerminal'
  )
    throw Error('terminal topic must name the opposite composition half');
  const auth = read(AUTH);
  if (
    !hasRustBearing(body(auth, 'may_consume'), '"guarded-by-capability"=>self.may_call(&s.method)')
  )
    throw Error('topic gate detached from caller method authority');
  if (!compact(body(auth, 'may_consume')).includes('spec.is_some_and'))
    throw Error('unknown scoped topic must fail closed');
  if (!compact(body(auth, 'topic_spec')).includes('vocabulary().topics'))
    throw Error('topic lookup detached from registry');
  const runtime = read(RUNTIME),
    masked = maskRust(runtime);
  const start = masked.indexOf('impl Core {');
  if (start < 0) throw Error('missing broker core');
  const open = masked.indexOf('{', start);
  const core = runtime.slice(open + 1, endOf(masked, open) - 1);
  const publish = compact(body(core, 'publish'));
  if (!hasRustBearing(publish, '!peer.identity.may_consume(&event.topic)'))
    throw Error('missing enqueue authority guard');
  const guardAt = publish.indexOf('!peer.identity.may_consume(&event.topic)');
  const continueAt = publish.indexOf('{continue;}', guardAt);
  const enqueueAt = publish.indexOf('.try_send(');
  const desyncAt = publish.indexOf('peer.desynced.lock()');
  if (guardAt < 0 || continueAt < guardAt || enqueueAt < continueAt || desyncAt < enqueueAt)
    throw Error('consume guard must precede enqueue and desync bookkeeping');
}
describe('terminal and replay composition bearings', () => {
  it('uses actual retained replay containment and capability-gated broker enqueue', () =>
    expect(() => verify()).not.toThrow());
  it('rejects wrong-half topic bindings, removed hops, unsafe open operands and sibling/comment decoys', () => {
    for (const [file, before, after] of [
      [
        CAPS,
        "guardReplaySession('replay.read', sessionId);",
        "guardReplaySession('replay.diff', sessionId);",
      ],
      [REPLAY, 'this.resolveInside(entry, p)', 'this.unchecked(entry, p)'],
      [REPLAY, "fs.openSync(abs, 'r')", "fs.openSync(p, 'r')"],
      [
        REPLAY,
        'const canonical = canonicalizePath(abs);',
        'const canonical = abs; // canonicalizePath(abs)',
      ],
      [REPLAY, 'if (!isWithin(canonical, root))', 'if (false)'],
      [
        AUTH,
        '"guarded-by-capability" => self.may_call(&s.method)',
        '"guarded-by-capability" => true',
      ],
      [
        RUNTIME,
        '|| !peer.identity.may_consume(&event.topic)',
        '// || !peer.identity.may_consume(&event.topic)',
      ],
    ]) {
      const source = fs.readFileSync(path.join(ROOT, file), 'utf8');
      expect(source).toContain(before);
      expect(() => verify(new Map([[file, source.replace(before, after)]])), before).toThrow();
    }
    const topics = structuredClone(vocabulary.topics);
    topics.find((row) => row.Pattern === 'pty.bytes.*')!.Method = 'agents.list';
    expect(() => verify(new Map(), topics)).toThrow('opposite composition half');
  });
});
