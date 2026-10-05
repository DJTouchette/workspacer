/** Actual source registrations, including optional services, force an authority
 * decision independently of captured vocabulary and running-graph inventory. */
import { describe, expect, it } from 'vitest';
import path from 'path';
import vocabulary from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
import reference from '../../../../../contracts/backend-capabilities.json';
import {
  registrations,
  rustSources,
  desktopRegistrations,
} from '../../../tests/support/capabilitySource';
import { compact, production } from '../../../tests/support/rustHttpSource';
import { productionFiles } from '../../../tests/support/rustProductionSource';
const ROOT = path.resolve(__dirname, '../../../../..');
interface Extra {
  file: string;
  actor: boolean;
  reason: string;
  gate: string[];
}
const EXTRA: Record<string, Extra> = {};
function extras(
  methods: string[],
  file: string,
  actor: boolean,
  reason: string,
  gate: string[],
): void {
  for (const name of methods)
    EXTRA[name] = { file: 'services/hub-rs/src/' + file, actor, reason, gate };
}
extras(
  ['fs.compareWrite'],
  'services/files.rs',
  true,
  'Conditional write under the same ambient authenticated file authority as fs.write. The shared filesystem dispatcher canonicalizes the absolute path before calling the locked writer; expected/contents are text and force changes only the comparison, never caller or execution authority.',
  ['call(method,params,&home)'],
);
extras(
  ['federation.resumePeer'],
  'federation.rs',
  true,
  'Resumes an intentionally paused federation link; only the actual local server owner may request a network wake.',
  [
    '!caller.authenticated_host',
    '!caller.trusted',
    'caller.federated',
    'caller.scope!="operator"',
    'bail!',
  ],
);
extras(
  ['internal.beginDelivery', 'internal.finishDelivery'],
  'services/manager_requests.rs',
  true,
  'Host-owned durable manager delivery bookkeeping; a raw scoped operator cannot invent receipt or delivery authority.',
  ['if!caller.authenticated_host||!caller.trusted{bail!'],
);
extras(
  ['internal.requestReceipt'],
  'services/manager_requests.rs',
  false,
  'Reads host-owned delivery state after actual owner authorization; neither caller identity nor arbitrary storage paths come from parameters.',
  ['if!caller.authenticated_host||!caller.trusted{bail!'],
);
extras(
  ['providerRelay.resume'],
  'provider_relay/mod.rs',
  true,
  'Explicitly resumes a paused upstream provider connection, through the actual host gate rather than ambient operator pairing.',
  ['anyhow::ensure!(caller.authenticated_host,'],
);
extras(
  ['providerRelay.status'],
  'provider_relay/mod.rs',
  false,
  'Reads private provider-relay connection status after actual host authorization; the handler does not manufacture caller authority.',
  ['anyhow::ensure!(caller.authenticated_host,'],
);
extras(
  ['plugins.list', 'plugins.manifests'],
  'plugins/mod.rs',
  false,
  'Full manifests can include host paths and runtime details, so the bus handler requires actual host identity before returning them.',
  ['ifmethod=="plugins.list"||method=="plugins.manifests"{if!caller.authenticated_host{bail!'],
);
extras(
  [
    'plugins.setSettings',
    'plugins.setEnabled',
    'plugins.reload',
    'plugins.paneToken',
    'plugins.revokePaneToken',
  ],
  'plugins/mod.rs',
  true,
  'These plugin lifecycle/settings mutations execute behind the actual owner gate; the sole exception in the branch is the read-only own-plugin settings method.',
  ['if!caller.authenticated_host&&!(method=="plugins.settings"&&caller.plugin_id==id){bail!'],
);
extras(
  ['plugins.settings'],
  'plugins/mod.rs',
  false,
  'A host may read settings; a plugin may read only the plugin ID proven by its authenticated connection, with sensitive values redacted by the settings service.',
  ['if!caller.authenticated_host&&!(method=="plugins.settings"&&caller.plugin_id==id){bail!'],
);
const DUPLICATES: Record<
  string,
  { files: string[]; reason: string; proofs: Record<string, string[]> }
> = {
  'plugins.tools': {
    files: ['plugins/mod.rs', 'runtime.rs', 'runtime.rs'],
    reason:
      'The runtime replaces the manager table entry with its catalog-ready barrier; absent managers expose the explicit empty catalog.',
    proofs: {
      'runtime.rs': [
        'ifletSome(manager)=&plugin_manager{options=crate::plugins::handlers(options,manager.clone());',
        'ready.wait_for(|ready|*ready)',
        '}else{options=options.handler("plugins.tools",|_,_|async{Ok(json!([]))});',
      ],
    },
  },
  'fleet.dispatchTargets': {
    files: ['services/remote_dispatch/paired.rs', 'services/remote_dispatch/runtime.rs'],
    reason:
      'The public empty-target fallback is installed first; configured paired dispatch replaces it only after creating the owned origin and workflow-backed service.',
    proofs: {
      'services/remote_dispatch/runtime.rs': [
        'options=options.handler("fleet.dispatchTargets"',
        'options=super::paired::Paired::install(options,origin.clone(),routes.clone(),hub.clone())?;',
      ],
    },
  },
};
function check(files: Map<string, string>): string[] {
  const rows = registrations(files),
    errors: string[] = [];
  const names = new Set(rows.map((row) => row.method));
  for (const required of ['layout.get', 'layout.set', 'agents.spawn', 'config.save'])
    if (!names.has(required)) errors.push(`required method missing ${required}`);
  for (const row of rows) {
    if (vocabulary.methods.includes(row.method)) continue;
    const extra = EXTRA[row.method];
    if (!extra) {
      errors.push(`unclassified registration ${row.method}`);
      continue;
    }
    if (extra.file !== row.file || extra.reason.length < 60 || typeof extra.actor !== 'boolean')
      errors.push(`invalid authority record ${row.method}`);
    for (const proof of extra.gate)
      if (!compact(row.handler).includes(proof))
        errors.push(`missing handler-bound gate ${row.method}: ${proof}`);
  }
  for (const name of Object.keys(EXTRA))
    if (!names.has(name)) errors.push(`stale authority record ${name}`);
  const duplicates = new Map<string, string[]>();
  for (const row of rows) {
    const paths = duplicates.get(row.method) || [];
    paths.push(row.file.replace('services/hub-rs/src/', ''));
    duplicates.set(row.method, paths);
  }
  for (const [name, paths] of duplicates)
    if (paths.length > 1) {
      const spec = DUPLICATES[name];
      if (!spec || JSON.stringify([...paths].sort()) !== JSON.stringify([...spec.files].sort()))
        errors.push(`unreviewed duplicate ${name}`);
    }
  for (const [name, spec] of Object.entries(DUPLICATES)) {
    if ((duplicates.get(name)?.length || 0) < 2) errors.push(`stale duplicate decision ${name}`);
    for (const [file, proofs] of Object.entries(spec.proofs))
      for (const proof of proofs)
        if (!compact(files.get('services/hub-rs/src/' + file) || '').includes(proof))
          errors.push(`missing override ownership ${name}: ${proof}`);
  }
  return errors;
}
describe('capability registration source', { timeout: 90000 }, () => {
  it('classifies all actual Rust declarations with exact extra-authority and duplicate-ownership decisions', () =>
    expect(check(rustSources(ROOT))).toEqual([]));
  it('pins conditional file writes to the shared path dispatcher and explicit current surface', () => {
    expect(reference.currentAdditions['fs.compareWrite'].authority).toBe('fs.write');
    expect(reference.full).toContain('fs.compareWrite');
    expect(reference.catalog).toContain('fs.compareWrite');
    expect(vocabulary.methods).not.toContain('fs.compareWrite'); // sealed historical vocabulary
    const files = rustSources(ROOT);
    const row = registrations(files).filter((row) => row.method === 'fs.compareWrite');
    expect(row).toHaveLength(1);
    expect(row[0].file).toBe('services/hub-rs/src/services/files.rs');
    expect(EXTRA['fs.compareWrite'].actor).toBe(true);
    const removed = new Map(files);
    const file = row[0].file;
    removed.set(
      file,
      files.get(file)!.replace('call(method, params, &home)', 'unguarded_write(params)'),
    );
    expect(check(removed)).toContain(
      'missing handler-bound gate fs.compareWrite: call(method,params,&home)',
    );
  });
  it('enumerates retained desktop registrations and keeps hub-owned methods out of the provider registry', () => {
    const rows = desktopRegistrations(ROOT);
    const duplicates = rows
      .map((row) => row.method)
      .filter((name, index, names) => names.indexOf(name) !== index);
    expect(duplicates).toEqual([]);
    expect(
      rows.filter((row) => !vocabulary.methods.includes(row.method)).map((row) => row.method),
    ).toEqual([]);
    expect(
      rows.filter((row) => reference.hub.includes(row.method)).map((row) => row.method),
    ).toEqual([]);
  });
  it('rejects unclassified sites, indirect lists, missing populations, name rebinding, gate loss and duplicate takeover', () => {
    const base = rustSources(ROOT);
    const added = (text: string): Map<string, string> =>
      new Map([...base, ['services/hub-rs/src/new_registry.rs', text]]);
    expect(
      check(added('fn install(o:Options){o.handler("fresh.actor",|_,_|async{Ok(())});}')),
    ).toContain('unclassified registration fresh.actor');
    expect(
      check(added('fn install(o:Options){o.handler("layout.set",|_,_|async{Ok(())});}')),
    ).toContain('unreviewed duplicate layout.set');
    expect(() => check(added('fn install(o:Options){o.handler(computed(),handler);}'))).toThrow(
      'unresolved',
    );
    expect(() =>
      check(
        added(
          'fn install(o:Options){for method in ["fresh.actor"] {let method=external; o.handler(method,handler);}}',
        ),
      ),
    ).toThrow('rebound');
    expect(() => registrations(new Map())).toThrow('population');
    expect(
      check(
        added(
          '// o.handler("fresh.actor",handler);\nfn f(){let text=r#"o.handler(computed(), handler)"#;}',
        ),
      ),
    ).toEqual([]);
    const changed = new Map(base),
      relay = 'services/hub-rs/src/provider_relay/mod.rs';
    changed.set(relay, changed.get(relay)!.replace('caller.authenticated_host,', 'true,'));
    expect(check(changed).some((error) => error.includes('missing handler-bound gate'))).toBe(true);
    const retired = new Map(base);
    retired.delete(relay);
    expect(check(retired)).toContain('stale authority record providerRelay.resume');
    const parent = 'services/hub-rs/src/fixture/mod.rs',
      child = 'services/hub-rs/src/fixture/test.rs';
    const tree = new Map([
      ...base,
      [parent, '#[cfg(test)] mod test;'],
      [child, 'fn install(o:Options){o.handler("fresh.actor",handler);}'],
    ]);
    expect(check(productionFiles(tree))).toEqual([]);
    tree.set(parent, 'mod test;');
    expect(check(productionFiles(tree))).toContain('unclassified registration fresh.actor');
    expect(
      check(
        new Map([...base].map(([file, text]) => [file, production(text.replace(/\n/g, '\r\n'))])),
      ),
    ).toEqual([]);
  });
  it('discovers desktop aliases, new modules and nested registrations without comments manufacturing evidence', () => {
    const file = 'apps/desktop/src/main/services/newRegistry.ts';
    const rows = desktopRegistrations(
      ROOT,
      new Map([
        [
          file,
          "import {registerCapability as install} from './hubClient'; const alias=install; alias('fresh.actor',()=>{});",
        ],
      ]),
    );
    expect(rows.some((row) => row.method === 'fresh.actor')).toBe(true);
    expect(() =>
      desktopRegistrations(
        ROOT,
        new Map([
          [
            file,
            "import {registerCapability} from './hubClient'; registerCapability(computed(),()=>{});",
          ],
        ]),
      ),
    ).toThrow('unresolved');
    const generated = 'apps/desktop/src/main/shared/desktopServices.generated.ts';
    const imported = desktopRegistrations(
      ROOT,
      new Map([
        [
          generated,
          "const methods={ownerMethods:['desktop.newReviewedMethod'],assetMethods:[]} as const; export default methods;",
        ],
      ]),
    );
    expect(imported.some((row) => row.method === 'desktop.newReviewedMethod')).toBe(true);
    expect(() =>
      desktopRegistrations(
        ROOT,
        new Map([
          [
            generated,
            'const methods={ownerMethods:compute(),assetMethods:[]} as const; export default methods;',
          ],
        ]),
      ),
    ).toThrow('unresolved');
    const normal = desktopRegistrations(ROOT).length;
    expect(
      desktopRegistrations(
        ROOT,
        new Map([
          [
            file,
            "// registerCapability('fake',()=>{});\nconst text=\"registerCapability('fake', handler)\";",
          ],
        ]),
      ).length,
    ).toBe(normal);
  });
});
