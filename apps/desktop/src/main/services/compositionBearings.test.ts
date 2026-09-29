/** Concrete method-to-guard edges complement the tier vocabulary. A guard in
 * another dispatcher branch or a comment does not prove a composition closed. */
import { describe, expect, it } from 'vitest';
import path from 'path';
import { registrations, rustSources } from '../../../tests/support/capabilitySource';
import { body, compact, endOf, maskRust } from '../../../tests/support/rustHttpSource';
const ROOT = path.resolve(__dirname, '../../../../..');
const PREFIX = 'services/hub-rs/src/';
function arm(source: string, method: string): string {
  const code = maskRust(source, false);
  const token = JSON.stringify(method) + ' =>';
  // Match only a method-specific braced arm, never a whole dispatcher body.
  const expression = new RegExp(
    JSON.stringify(method).replace(/\./g, '\\.') + '\\s*=>\\s*\\{',
    'g',
  );
  const matches = [...code.matchAll(expression)];
  if (matches.length !== 1) throw Error(`expected one branch ${token}`);
  const open = code.indexOf('{', matches[0].index);
  return source.slice(open + 1, endOf(maskRust(source), open) - 1);
}
function verify(files: Map<string, string>): void {
  const rows = registrations(files);
  const source = (file: string): string => {
    const text = files.get(PREFIX + file);
    if (!text) throw Error(`missing source ${file}`);
    return text;
  };
  const has = (text: string, edge: string): void => {
    if (!compact(text).includes(edge)) throw Error(`missing bearing ${edge}`);
  };
  const registered = (method: string, file: string, edge: string): void => {
    const matches = rows.filter((row) => row.method === method && row.file === PREFIX + file);
    if (matches.length !== 1) throw Error(`missing unique registration ${method}`);
    has(matches[0].handler, edge);
  };
  for (const method of ['jobs.upsert', 'jobs.run'])
    registered(method, 'services/jobs.rs', 'service.call(&caller,method,params)');
  const jobs = body(source('services/jobs.rs'), 'call');
  const gate = 'if!caller.authenticated_host||!caller.trusted||caller.scope!="operator"{bail!';
  has(jobs, gate);
  if (compact(jobs).indexOf(gate) > compact(jobs).indexOf('matchmethod{'))
    throw Error('jobs owner gate moved behind method dispatch');

  registered('layout.set', 'services/mod.rs', 'service.set(&caller,params)');
  const layout = source('services/layout.rs');
  has(body(layout, 'set'), 'if!caller.trusted{scrub_boot_document(&mutdata);}');
  has(body(layout, 'scrub_boot_document'), 'scrub_document(value,true);');
  has(body(layout, 'scrub_saved_document'), 'scrub_document(value,false);');
  const scrub = body(layout, 'scrub_document');
  has(scrub, 'agent.remove(key)');
  for (const key of [
    'skipPermissions',
    'permissionMode',
    'profileId',
    'mcpItemIds',
    'launchIntegrationId',
  ])
    has(scrub, JSON.stringify(key));
  has(scrub, 'panes');
  has(scrub, 'pane.remove(key)');
  for (const key of ['shell', 'initialCommand', 'pluginId']) has(scrub, JSON.stringify(key));
  const stores = source('services/stores.rs');
  for (const [method, variable] of [
    ['layouts.save', 'layout'],
    ['sessions.save', 'data'],
  ]) {
    registered(method, 'services/stores.rs', 'service.call(method,params)');
    const branch = arm(body(stores, 'call'), method);
    const edge = `scrub_saved_document(&mut${variable});`;
    has(branch, edge);
    if (compact(branch).indexOf(edge) > compact(branch).indexOf('atomic_bytes('))
      throw Error(`scrub follows persistence ${method}`);
  }
  registered(
    'push.subscribe',
    'services/push/mod.rs',
    '"push.subscribe"=>service.subscribe(caller,params)',
  );
  const push = source('services/push/mod.rs');
  has(body(push, 'subscribe'), 'crypto::endpoint(&row.endpoint)?;');
  const crypto = source('services/push/crypto.rs');
  has(body(crypto, 'endpoint'), 'url.scheme()=="https"');
  has(body(crypto, 'endpoint'), 'public_ip(ip)');
  has(body(crypto, 'request'), 'endpoint(&subscription.endpoint)?;');
  for (const check of [
    '!ip.is_loopback()',
    '!ip.is_private()',
    '!ip.is_unspecified()',
    '!ip.is_link_local()',
    'ip.to_ipv4_mapped()',
    'public_ip(v4.into())',
  ])
    has(body(crypto, 'public_ip'), check);
}
describe('composition source bearings', () => {
  it('binds jobs, layout, saved documents and push closure to their actual concrete registration and call chains', () => {
    expect(() => verify(rustSources(ROOT))).not.toThrow();
  });
  it('rejects missing call edges, sibling-arm proofs, dispatch-late guards and decoy comments', () => {
    const base = rustSources(ROOT);
    const mutations: [string, string, string][] = [
      [
        'services/jobs.rs',
        'service.call(&caller, method, params)',
        'service.other(&caller, method, params)',
      ],
      ['services/jobs.rs', '!caller.authenticated_host', 'false'],
      ['services/mod.rs', 'service.set(&caller, params)', 'service.other(&caller, params)'],
      ['services/layout.rs', 'scrub_document(value, true);', '/* scrub_document(value, true); */'],
      ['services/layout.rs', '"launchIntegrationId",', '"differentKey",'],
      [
        'services/stores.rs',
        'scrub_saved_document(&mut layout);',
        '// scrub_saved_document(&mut layout);',
      ],
      [
        'services/stores.rs',
        'scrub_saved_document(&mut data);',
        '// scrub_saved_document(&mut data);',
      ],
      [
        'services/push/mod.rs',
        'crypto::endpoint(&row.endpoint)?;',
        '// crypto::endpoint(&row.endpoint)?;',
      ],
      ['services/push/crypto.rs', 'public_ip(ip),', 'true,'],
      [
        'services/push/crypto.rs',
        'endpoint(&subscription.endpoint)?',
        'url::Url::parse(&subscription.endpoint)?',
      ],
    ];
    for (const [file, before, after] of mutations) {
      const changed = new Map(base),
        text = changed.get(PREFIX + file)!;
      expect(text, before).toContain(before);
      changed.set(PREFIX + file, text.replace(before, after));
      expect(() => verify(changed), `${file}: ${before}`).toThrow();
    }
    const changed = new Map(base),
      file = PREFIX + 'services/stores.rs';
    changed.set(file, changed.get(file)!.replace('"layouts.save" => {', '"layouts.other" => {'));
    expect(() => verify(changed)).toThrow('branch');
  });
});
