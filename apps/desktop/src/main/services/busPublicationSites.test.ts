import { describe, expect, it } from 'vitest';
import fs from 'fs';
import path from 'path';
import ts from 'typescript';
import vocabulary from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
import { body, compact, maskRust, production } from '../../../tests/support/rustHttpSource';
import { argumentsAt, productionFiles } from '../../../tests/support/rustProductionSource';
const ROOT = path.resolve(__dirname, '../../../../..');
function sources(directory: string): Map<string, string> {
  const result = new Map<string, string>();
  const walk = (dir: string): void => {
    for (const entry of fs.readdirSync(path.join(ROOT, dir), { withFileTypes: true })) {
      if (entry.isSymbolicLink())
        throw Error(`source symlink is not accounted for: ${dir}/${entry.name}`);
      const name = `${dir}/${entry.name}`;
      if (entry.isDirectory()) walk(name);
      else if (entry.name.endsWith('.rs'))
        result.set(name, fs.readFileSync(path.join(ROOT, name), 'utf8'));
    }
  };
  walk(directory);
  return productionFiles(result);
}
function resolved(file: string, source: string, expression: string): string[] | undefined {
  const value = compact(expression);
  const literal = /^"([\w.]+)"$/.exec(value);
  if (literal) return [literal[1]];
  if (file.endsWith('/mcp/ui.rs') && value === 'topic') {
    const mapping = body(source, 'topic');
    if (!compact(mapping).includes('_=>returnNone')) throw Error('UI topic mapper lost refusal');
    const names = [...mapping.matchAll(/=>\s*"([\w.]+)"/g)].map((match) => match[1]);
    if (names.length !== 6) throw Error('UI topic mapper shape changed');
    return names;
  }
  if (
    file.endsWith('/plugins/mod.rs') &&
    value === 'topic' &&
    compact(source).includes('lettopic=format!("sidecar.{}",status.state.name());')
  )
    return ['sidecar.*'];
  if (
    file.endsWith('/runtime.rs') &&
    value === 'topic.clone()' &&
    compact(source).includes(
      'ifletSome(id)=topic.strip_prefix("agent.conversation."){Event::new(topic.clone(),',
    )
  )
    return ['agent.conversation.*'];
  if (
    file.endsWith('/services/live_streams.rs') &&
    value === 'format!("{PREFIX}{}",delivery.session)'
  ) {
    const prefix = /const\s+PREFIX\s*:\s*&str\s*=\s*"([\w.]+)"/.exec(source)?.[1];
    return prefix ? [prefix + '*'] : undefined;
  }
  if (file.endsWith('/services/terminals.rs') && value === 'format!("pty.bytes.{session}")')
    return ['pty.bytes.*'];
  if (
    file.endsWith('/services/sessions.rs') &&
    value === 'topic' &&
    compact(body(source, 'workflow_update')).includes(
      'letSome(topic)=event["type"].as_str()else{continue;};',
    )
  ) {
    // Private workflow observer messages carry events from the finite telemetry
    // builder; arbitrary daemon/caller event names never enter this path.
    const telemetry = production(
      fs.readFileSync(
        path.join(ROOT, 'services/hub-rs/src/services/workflow_artifacts/telemetry.rs'),
        'utf8',
      ),
    );
    const names = [...telemetry.matchAll(/"(workflow\.[\w.]+)"/g)].map((match) => match[1]);
    return [...new Set(names)];
  }
  return undefined;
}
const DYNAMIC_CONSTRUCTORS: Record<string, number> = {
  'mcp/ui.rs: topic': 1,
  'plugins/mod.rs: topic': 1,
  'runtime.rs: topic.clone()': 1,
  'services/live_streams.rs: format!("{PREFIX}{}",delivery.session)': 2,
  'services/sessions.rs: topic': 1,
  'services/terminals.rs: format!("pty.bytes.{session}")': 1,
};
function enumerate(files: Map<string, string>): Map<string, Set<string>> {
  const topics = new Map<string, Set<string>>();
  let count = 0;
  const dynamic = new Map<string, number>();
  for (const [file, source] of files) {
    const code = maskRust(source);
    if (/\bEvent\s+as\s+\w+/.test(code)) throw Error(`unaccounted event alias ${file}`);
    for (const literal of code.matchAll(/\bEvent\s*\{/g)) {
      if (
        !/\b(?:struct|impl)\s*$/.test(code.slice(Math.max(0, literal.index! - 20), literal.index))
      )
        throw Error(`unaccounted event struct ${file}`);
    }
    for (const call of code.matchAll(/\bEvent\s*::\s*new\s*\(/g)) {
      const [topic] = argumentsAt(source, code.indexOf('(', call.index));
      if (!/^"[\w.]+"$/.test(compact(topic))) {
        const key = file.replace('services/hub-rs/src/', '') + ': ' + compact(topic);
        if (!DYNAMIC_CONSTRUCTORS[key]) throw Error(`unresolved constructor ${key}`);
        dynamic.set(key, (dynamic.get(key) || 0) + 1);
      }
      const names = resolved(file, source, topic);
      if (!names?.length) throw Error(`unresolved constructor ${file}: ${topic}`);
      count++;
      for (const name of names) {
        const owners = topics.get(name) || new Set<string>();
        owners.add(file);
        topics.set(name, owners);
      }
    }
  }
  for (const [key, expected] of Object.entries(DYNAMIC_CONSTRUCTORS))
    if (dynamic.get(key) !== expected) throw Error(`stale dynamic constructor ${key}`);
  if (count < 35) throw Error('constructor population collapsed');
  return topics;
}
function classified(pattern: string): (typeof vocabulary.topics)[number] | undefined {
  const probe = pattern.endsWith('*') ? pattern.slice(0, -1) + 'probe' : pattern;
  return (
    vocabulary.topics.find((row) => row.Pattern === probe) ||
    vocabulary.topics.filter(
      (row) => row.Pattern.endsWith('*') && probe.startsWith(row.Pattern.slice(0, -1)),
    )[0]
  );
}
function electronTopics(): Map<string, Set<string>> {
  const result = new Map<string, Set<string>>();
  let calls = 0;
  const walk = (directory: string): void => {
    for (const entry of fs.readdirSync(path.join(ROOT, directory), { withFileTypes: true })) {
      const file = `${directory}/${entry.name}`;
      if (entry.isSymbolicLink()) throw Error(`unaccounted source symlink ${file}`);
      if (entry.isDirectory()) {
        walk(file);
        continue;
      }
      if (!entry.name.endsWith('.ts') || entry.name.endsWith('.test.ts')) continue;
      const raw = fs.readFileSync(path.join(ROOT, file), 'utf8');
      // Aliased imports still carry the exported name in their import specifier.
      if (!raw.includes('publishToHub')) continue;
      const source = ts.createSourceFile(file, raw, ts.ScriptTarget.Latest, true);
      const names = new Set(['publishToHub']);
      const aliases = (node: ts.Node): void => {
        if (ts.isImportSpecifier(node) && node.propertyName?.text === 'publishToHub')
          names.add(node.name.text);
        ts.forEachChild(node, aliases);
      };
      aliases(source);
      const visit = (node: ts.Node): void => {
        if (
          ts.isCallExpression(node) &&
          ((ts.isIdentifier(node.expression) && names.has(node.expression.text)) ||
            (ts.isPropertyAccessExpression(node.expression) &&
              node.expression.name.text === 'publishToHub'))
        ) {
          calls++;
          const arg = node.arguments[0];
          let topics: string[] | undefined;
          if (arg && ts.isObjectLiteralExpression(arg)) {
            const type = arg.properties.find(
              (property) =>
                ts.isPropertyAssignment(property) &&
                property.name.getText(source).replace(/['"]/g, '') === 'type',
            );
            if (type && ts.isPropertyAssignment(type)) {
              const value = type.initializer;
              if (ts.isStringLiteralLike(value)) topics = [value.text];
              else if (ts.isTemplateExpression(value) && value.head.text === 'pty.bytes.')
                topics = ['pty.bytes.*'];
              else if (
                ts.isTemplateExpression(value) &&
                value.getText(source) === '`workflow.${run.status}`' &&
                raw.includes("run.status === 'completed' || run.status === 'failed'")
              )
                topics = ['workflow.completed', 'workflow.failed'];
            }
          }
          // Renderer-to-host IPC explicitly carries arbitrary envelopes. The
          // broker's authenticated host/plugin publish gate owns that input.
          if (
            !topics &&
            file === 'apps/desktop/src/main/ipc.ts' &&
            arg?.getText(source) === 'ev' &&
            raw.includes('IPC.HUB_PUBLISH,')
          )
            topics = [];
          if (!topics)
            throw Error(`unresolved Electron publication ${file}: ${node.getText(source)}`);
          for (const topic of topics) {
            const owners = result.get(topic) || new Set<string>();
            owners.add(file);
            result.set(topic, owners);
          }
        }
        ts.forEachChild(node, visit);
      };
      visit(source);
    }
  };
  walk('apps/desktop/src/main');
  if (calls < 9 || result.size < 7) throw Error('Electron publication population collapsed');
  return result;
}
// Exact variable-envelope sites are reviewed separately from literal factories.
// Counts close the list in both directions: adding a second occurrence requires
// a new review; deleting a producer cannot leave a stale exception behind.
const PASSTHROUGH: Record<string, { count: number; reason: string; proof: string[] }> = {
  'federation/config.rs: peers': {
    count: 1,
    reason: 'Configuration watch publication, not an event-bus envelope.',
    proof: ['publication.publish(peers)'],
  },
  'federation.rs: event': {
    count: 1,
    reason: 'Remote envelope is filtered and namespaced by forward_event before publication.',
    proof: ['forward_event(', 'hub.publish_wait(event)'],
  },
  'mcp.rs: event': {
    count: 2,
    reason:
      'Finite UI event mapper, after tool and credential tier authorization; local and upstream routes share mapper.',
    proof: ['ifletSome(event)=ui::event(', 'UI actions require triage or operator authority'],
  },
  'runtime.rs: event': {
    count: 3,
    reason:
      'Actor input: checked client envelope, trusted host command, or generation-checked live-stream delivery.',
    proof: [
      'self.peers[&id].identity.may_publish(&event.topic)',
      'streams.accept_delivery(delivery)',
      'Command::Publish(event)=>core.publish(event)',
    ],
  },
  'services/external_claudemon.rs: event': {
    count: 2,
    reason:
      'SSE parser maps daemon status events through the closed map_event constructor, including end-of-stream flush.',
    proof: ['parser.push(&chunk)?', 'parser.finish()?', 'fnmap_event('],
  },
  'services/live_streams.rs: delivery': {
    count: 1,
    reason:
      'Typed delivery is validated by actor accept_delivery before creating the classified conversation/statusline event.',
    proof: ['hub.publish_live_stream(delivery)'],
  },
  'services/remote_dispatch/paired.rs: event': {
    count: 3,
    reason:
      'Three paired-proxy snapshot constructors; remote lineage metadata is attached after construction.',
    proof: ['Event::new("agent.snapshot","brain",'],
  },
  'services/sessions.rs: row.clone()': {
    count: 1,
    reason:
      'Snapshot helper receives a row, enriches and constructs agent.snapshot; it is not an arbitrary envelope.',
    proof: ['Event::new("agent.snapshot","brain",self.enrich(row))'],
  },
  'services/sessions.rs: next': {
    count: 1,
    reason:
      'Workflow metadata update publishes its merged snapshot through the same snapshot helper.',
    proof: ['self.publish(next).await?'],
  },
  'services/sessions.rs: row': {
    count: 1,
    reason: 'Seed/update path publishes a snapshot through the classified snapshot constructor.',
    proof: ['self.publish(row).await?'],
  },
  'services/terminals.rs: event': {
    count: 1,
    reason:
      'Terminal byte publisher constructs the fixed pty.bytes session family then applies backpressure.',
    proof: ['Event::new(format!("pty.bytes.{session}"),'],
  },
};
function passthroughs(files: Map<string, string>): string[] {
  const sites: string[] = [];
  for (const [file, source] of files) {
    const code = maskRust(source);
    for (const call of code.matchAll(/\.publish(?:_wait|_live_stream)?\s*\(/g)) {
      const [event] = argumentsAt(source, code.indexOf('(', call.index));
      if (/\bEvent\s*::\s*new\s*\(/.test(maskRust(event))) continue;
      sites.push(`${file}: ${compact(event)}`);
    }
  }
  return sites;
}
function validatePublications(files: Map<string, string>): void {
  for (const [topic, owners] of enumerate(files))
    if (!classified(topic)) throw Error(`unclassified ${topic}: ${[...owners]}`);
  const counts = new Map<string, number>();
  for (const site of passthroughs(files)) {
    const key = site.replace('services/hub-rs/src/', '');
    if (!PASSTHROUGH[key]) throw Error(`unreviewed passthrough ${key}`);
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  for (const [key, entry] of Object.entries(PASSTHROUGH)) {
    if (counts.get(key) !== entry.count) throw Error(`stale passthrough ${key}`);
    const source = compact(files.get('services/hub-rs/src/' + key.split(': ')[0])!);
    for (const proof of entry.proof)
      if (!source.includes(proof)) throw Error(`missing proof ${key}: ${proof}`);
  }
}
describe('bus publication sites', { timeout: 90_000 }, () => {
  it('enumerates actual constructors and derives sidecar host-only authority from their source', () => {
    const topics = enumerate(sources('services/hub-rs/src'));
    let hostOnly = 0;
    for (const [topic, files] of topics) {
      const row = classified(topic);
      expect(row, `unclassified production topic ${topic}: ${[...files]}`).toBeDefined();
      if ([...files].every((file) => file.includes('/plugins/'))) {
        hostOnly++;
        expect(row!.Disposition, `sidecar control plane ${topic}`).toBe('host-only');
      }
    }
    expect(hostOnly).toBeGreaterThanOrEqual(5);
  });
  it('accounts for each publication passthrough without stale exemptions', () => {
    validatePublications(sources('services/hub-rs/src'));
  });
  it('cannot hide new modules, unresolved topics, renamed envelopes or stale exceptions', () => {
    const baseline = sources('services/hub-rs/src');
    expect(() =>
      validatePublications(
        new Map([...baseline].map(([file, source]) => [file, source.replace(/\n/g, '\r\n')])),
      ),
    ).not.toThrow();
    const injected = (source: string): Map<string, string> =>
      new Map([...baseline, ['services/hub-rs/src/new_publisher.rs', source]]);
    expect(() =>
      validatePublications(
        injected('fn f(){hub.publish(Event::new("unknown.topic", "hub", value));}'),
      ),
    ).toThrow('unclassified');
    expect(() =>
      validatePublications(injected('fn f(){hub.publish(Event::new(chosen, "hub", value));}')),
    ).toThrow('unresolved');
    expect(() => validatePublications(injected('fn f(){hub.publish(event);}'))).toThrow(
      'unreviewed',
    );
    expect(() =>
      validatePublications(
        injected('fn f(){let event=Event { topic: input, ..Default::default() };}'),
      ),
    ).toThrow('event struct');
    expect(() => validatePublications(injected('use crate::protocol::Event as Other;'))).toThrow(
      'event alias',
    );
    expect(() =>
      validatePublications(
        injected('// Event::new(chosen,"hub", value);\nfn f(){let decoy=r#"Event::new(chosen)"#;}'),
      ),
    ).not.toThrow();
    const changed = new Map(baseline),
      file = 'services/hub-rs/src/services/terminals.rs';
    changed.set(
      file,
      changed
        .get(file)!
        .replace('.publish_wait(event)', '.publish_wait(Event::new("pty.exit", "brain", value))'),
    );
    expect(() => validatePublications(changed)).toThrow('stale passthrough');
    const parent = 'services/hub-rs/src/fixture/mod.rs',
      child = 'services/hub-rs/src/fixture/tests.rs';
    const raw = new Map([
      ...baseline,
      [parent, '#[cfg(test)] mod tests;'],
      [child, 'fn f(){hub.publish(Event::new("unknown.topic", "hub", value));}'],
    ]);
    expect(() => validatePublications(productionFiles(raw))).not.toThrow();
    raw.set(parent, 'mod tests;');
    expect(() => validatePublications(productionFiles(raw))).toThrow('unclassified');
    raw.set(parent, '// #[cfg(test)] mod tests;');
    expect(() => validatePublications(productionFiles(raw))).toThrow('unclassified');
    raw.set(parent, '#[cfg(test)] mod tests;\n#[path="tests.rs"] mod also_live;');
    expect(() => validatePublications(productionFiles(raw))).toThrow('unclassified');
  });
  it('classifies retained Electron publication sites with a syntax-tree scan', () => {
    const electron = electronTopics();
    for (const [topic, files] of electron)
      expect(classified(topic), `${topic}: ${[...files]}`).toBeDefined();
    const produced = [...enumerate(sources('services/hub-rs/src')).keys(), ...electron.keys()];
    const stale = vocabulary.topics
      .filter(
        (row) =>
          !produced.some(
            (topic) =>
              topic === row.Pattern ||
              (topic.endsWith('*') && row.Pattern.startsWith(topic.slice(0, -1))) ||
              (row.Pattern.endsWith('*') && topic.startsWith(row.Pattern.slice(0, -1))),
          ),
      )
      .map((row) => row.Pattern);
    expect(stale, 'classified rows without an actual producer').toEqual([]);
  });
});
