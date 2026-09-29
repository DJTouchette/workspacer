/** Event authority invariants, independent of runtime delivery smoke. Production
 * publication-site completeness is checked separately; a registry checking its
 * own rows alone cannot prove that every published topic was classified. */
import { describe, expect, it } from 'vitest';
import fs from 'fs';
import path from 'path';
import vocabulary from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
import { body, compact, maskRust, production } from '../../../tests/support/rustHttpSource';
const ROOT = path.resolve(__dirname, '../../../../..');
type Topic = (typeof vocabulary.topics)[number];
const read = (name: string): string => production(fs.readFileSync(path.join(ROOT, name), 'utf8'));
function validate(topics: Topic[], methods: string[], relay: string): string[] {
  const errors: string[] = [];
  const patterns = new Set<string>();
  if (topics.length < 36) errors.push('topic population collapsed');
  for (const row of topics) {
    if (patterns.has(row.Pattern)) errors.push(`duplicate ${row.Pattern}`);
    patterns.add(row.Pattern);
    if (!/^[A-Za-z][\w.]*(?:\*)?$/.test(row.Pattern)) errors.push(`invalid pattern ${row.Pattern}`);
    if (row.Reason.trim().length < 60) errors.push(`missing reason ${row.Pattern}`);
    if (!['guarded-by-capability', 'host-only', 'open-by-decision'].includes(row.Disposition))
      errors.push(`invalid disposition ${row.Pattern}`);
    if (
      row.Disposition === 'guarded-by-capability'
        ? !methods.includes(row.Method)
        : row.Method !== ''
    )
      errors.push(`invalid consumer ${row.Pattern}`);
    if (row.Publisher && (!methods.includes(row.Publisher) || row.Publisher.includes('*')))
      errors.push(`invalid publisher ${row.Pattern}`);
  }
  for (const disposition of ['guarded-by-capability', 'host-only', 'open-by-decision']) {
    if (!topics.some((row) => row.Disposition === disposition))
      errors.push(`unused disposition ${disposition}`);
  }
  for (const topic of [
    'layout.changed',
    'plugin.settings.changed',
    'plugin.log',
    'agent.state_changed',
    'node.state_changed',
    'plugin.install.progress',
  ]) {
    const row = topics.find((row) => row.Pattern === topic);
    if (!row || row.Publisher) errors.push(`forgery publisher ${topic}`);
  }
  // This is the actual retained provider relay's subscription declaration, not
  // a copied list of expected feeds. Both static and demand-driven feeds count.
  const code = maskRust(relay);
  const start = code.match(/const\s+TOPICS\s*:\s*&\[&str\]\s*=\s*&\[/)?.index;
  if (start === undefined) return [...errors, 'relay topic declaration missing'];
  const open = code.indexOf('=', start);
  const end = code.indexOf('];', open);
  if (end < 0) return [...errors, 'relay topic declaration unterminated'];
  const names = [...relay.slice(open, end).matchAll(/"([\w.*]+)"/g)].map((match) => match[1]);
  if (names.length < 11) errors.push('relay feed population collapsed');
  const allowed = body(relay, 'event_allowed');
  if (!compact(allowed).includes('topic.starts_with("agent.conversation.")'))
    errors.push('conversation demand branch missing');
  names.push('agent.conversation.*');
  for (const name of names) {
    const row = topics.find((row) => row.Pattern === name);
    if (!row?.Publisher) errors.push(`mute provider feed ${name}`);
  }
  return errors;
}
const RELAY = 'services/hub-rs/src/provider_relay/methods.rs';
describe('bus event authority registry', () => {
  it('classifies consumers and actual provider feeds without opening forgery publishers', () => {
    expect(validate(vocabulary.topics, vocabulary.methods, read(RELAY))).toEqual([]);
  });
  it('refuses malformed rows, stale feed declarations, unknown methods and publisher widening', () => {
    const run = (edit: (rows: Topic[]) => void): string[] => {
      const rows = structuredClone(vocabulary.topics);
      edit(rows);
      return validate(rows, vocabulary.methods, read(RELAY));
    };
    expect(run((rows) => rows.push({ ...rows[0] }))).not.toEqual([]);
    expect(
      run((rows) => {
        rows[0].Reason = '';
      }),
    ).not.toEqual([]);
    expect(
      run((rows) => {
        rows[0].Method = 'unknown.method';
      }),
    ).not.toEqual([]);
    expect(
      run((rows) => {
        rows[0].Publisher = 'sessions.*';
      }),
    ).not.toEqual([]);
    expect(
      run((rows) => {
        rows.find((row) => row.Pattern === 'layout.changed')!.Publisher = 'layout.get';
      }),
    ).not.toEqual([]);
    expect(
      run((rows) => {
        rows.find((row) => row.Pattern === 'agent.snapshot')!.Publisher = '';
      }),
    ).not.toEqual([]);
    expect(
      validate(
        vocabulary.topics,
        vocabulary.methods,
        read(RELAY).replace('const TOPICS', 'const RENAMED'),
      ),
    ).not.toEqual([]);
    expect(
      validate(
        vocabulary.topics,
        vocabulary.methods,
        read(RELAY).replace('"agent.snapshot",', '"unknown.feed",'),
      ),
    ).not.toEqual([]);
    expect(
      validate(
        vocabulary.topics,
        vocabulary.methods,
        '// const TOPICS: &[&str] = &["agent.snapshot"];',
      ),
    ).not.toEqual([]);
  });
});
