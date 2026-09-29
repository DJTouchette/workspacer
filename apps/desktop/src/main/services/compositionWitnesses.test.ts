import { describe, expect, it } from 'vitest';
import path from 'path';
import vocabulary from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
import tiers from '../../../../../services/hub-rs/tests/fixtures/authorization-compositions.json';
import reference from '../../../../../contracts/backend-capabilities.json';
import {
  claims,
  actors,
  parameterDecisions,
  type Claim,
} from '../../../tests/support/compositionDecisions';
import { guardVerifier } from '../../../tests/support/compositionWitnesses';
import {
  desktopRegistrations,
  registrations,
  rustSources,
} from '../../../tests/support/capabilitySource';
const ROOT = path.resolve(__dirname, '../../../../..');
const NONE = [
  'claude.signal',
  'claude.handoffBrief',
  'claude.handoffAgentBrief',
  'fs.read',
  'fs.write',
  'search.project',
  'providers.listModels',
];
// Closed vocabulary: new exemptions must change both a claim and this verifier.
const INVERSE = [
  'save:delete',
  'save:remove',
  'add:remove',
  'subscribe:unsubscribe',
  'subscribe:revoke',
  'watch:unwatch',
  'stage:unstage',
  'approve:gate',
  'upsert:remove',
];
function check(
  record: Record<string, Claim>,
  files = rustSources(ROOT),
  params = parameterDecisions,
): void {
  const same = (actual: string[], expected: string[], what: string): void => {
    if (JSON.stringify([...actual].sort()) !== JSON.stringify([...expected].sort()))
      throw Error(what + ' population differs');
  };
  same(Object.keys(record), actors, 'considered actor');
  same(actors, tiers.actors, 'classification actor');
  const names = new Set([
    ...registrations(files).map((row) => row.method),
    ...desktopRegistrations(ROOT, files).map((row) => row.method),
  ]);
  const retired = Object.keys(reference.architecturalRetirements);
  const halves = new Set(tiers.pairs.flatMap((pair) => [pair.a, pair.b]));
  const guard = guardVerifier(ROOT, files),
    none: string[] = [],
    inverses = new Set<string>(),
    kinds = new Map<string, number>();
  const usedParams = new Map<string, Set<string>>();
  let written = 0,
    blank = 0;
  for (const [method, claim] of Object.entries(record)) {
    if (!names.has(method) && !retired.includes(method))
      throw Error('unregistered claim ' + method);
    if (!claim.reason.trim()) {
      if (!halves.has(method) || claim.witnesses.length)
        throw Error('invalid recorded half ' + method);
      blank++;
      continue;
    }
    if (halves.has(method) || claim.reason.trim().length < 60 || !claim.witnesses.length)
      throw Error('missing inert reason/witness ' + method);
    written++;
    for (const witness of claim.witnesses) {
      kinds.set(witness.kind, (kinds.get(witness.kind) || 0) + 1);
      switch (witness.kind) {
        case 'none':
          if (!NONE.includes(method) || !claim.reason.includes('NOTHING HERE IS MACHINE-CHECKED'))
            throw Error('unreviewed prose-only claim ' + method);
          none.push(method);
          break;
        case 'guard':
        case 'rust-git':
          if (!witness.guard) throw Error('missing guard ' + method);
          const named =
            witness.kind === 'rust-git'
              ? 'guardGitCwd'
              : witness.guard === 'asset-containment'
                ? 'canonicalize'
                : witness.guard;
          if (!claim.reason.includes(named)) throw Error('guard absent from reason ' + method);
          guard(method, witness.guard);
          break;
        case 'narrows': {
          const twin = witness.widens;
          if (!twin || !record[twin]) throw Error('unknown widening twin ' + method);
          const a = twin.split('.'),
            b = method.split('.'),
            pair = a.pop() + ':' + b.pop();
          if (
            a.join('.') !== b.join('.') ||
            !INVERSE.includes(pair) ||
            !claim.reason.includes(twin)
          )
            throw Error('invalid inverse ' + method);
          inverses.add(pair);
          break;
        }
        case 'topic': {
          const row = vocabulary.topics.find((row) => row.Pattern === witness.topic);
          if (
            !row ||
            row.Disposition !== 'guarded-by-capability' ||
            row.Method !== method ||
            !claim.reason.includes(witness.topic!)
          )
            throw Error('invalid output topic ' + method);
          break;
        }
        case 'params':
          if (!witness.params?.length) throw Error('empty parameter witness ' + method);
          for (const name of witness.params) {
            const used = usedParams.get(method) || new Set<string>();
            used.add(name);
            usedParams.set(method, used);
            const decision = params[method]?.[name];
            if (
              !decision ||
              ![
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
              ].includes(decision.kind) ||
              decision.reason.length < 30
            )
              throw Error('unclassified acting parameter ' + method + '.' + name);
            // Dotted config fields may be named by their exact spelling or as
            // part of the parent's prose list, matching the reference record.
            if (
              !(name.length <= 4
                ? claim.reason.includes('`' + name + '`')
                : new RegExp('\\b' + name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '\\b').test(
                    claim.reason,
                  ))
            )
              throw Error('parameter absent from reason ' + method + '.' + name);
          }
          break;
        default:
          throw Error('unknown witness kind');
      }
    }
  }
  same(Object.keys(params), [...usedParams.keys()], 'parameter method decision');
  for (const [method, used] of usedParams)
    same(Object.keys(params[method]), [...used], 'parameter decision ' + method);
  same(none, NONE, 'prose exception');
  same([...inverses], INVERSE, 'inverse exception');
  if (written < 30 || blank === 0 || (kinds.get('guard') || 0) < 15)
    throw Error('witness floor collapsed');
  for (const kind of ['guard', 'narrows', 'params', 'topic'])
    if (!kinds.has(kind)) throw Error('unused witness kind ' + kind);
}
describe('composition inert witnesses', () => {
  it('covers every classified actor with live source, exact inverse/topic/parameter decisions or a named prose exception', () =>
    expect(() => check(claims)).not.toThrow());
  it('rejects unknown actors, missing witnesses, new prose exemptions, stale inverse exemptions and parameter laundering', () => {
    const mutate = (change: (record: Record<string, Claim>) => void): void => {
      const copy = structuredClone(claims);
      change(copy);
      expect(() => check(copy)).toThrow();
    };
    mutate((copy) => {
      delete copy['fs.watch'];
    });
    mutate((copy) => {
      copy['new.actor'] = { reason: 'x'.repeat(100), witnesses: [{ kind: 'none' }] };
    });
    mutate((copy) => {
      copy['fs.watch'].witnesses = [];
    });
    mutate((copy) => {
      copy['fs.watch'].witnesses = [{ kind: 'none' }];
      copy['fs.watch'].reason += ' NOTHING HERE IS MACHINE-CHECKED';
    });
    mutate((copy) => {
      copy['fs.read'].witnesses = [{ kind: 'params', params: ['path'] }];
    });
    mutate((copy) => {
      copy['fs.unwatch'].witnesses.find((w) => w.kind === 'narrows')!.widens = 'git.stage';
    });
    mutate((copy) => {
      copy['fs.watch'].witnesses.find((w) => w.kind === 'topic')!.topic = 'pty.bytes.*';
    });
    mutate((copy) => {
      copy['config.save'].witnesses[0].params = ['inventedCommand'];
    });
    mutate((copy) => {
      copy['library.save'].witnesses[0].guard = 'guardGitCwd';
    });
    const params = structuredClone(parameterDecisions);
    params['config.save']['terminal.shell'].kind = 'inert';
    expect(() => check(claims, rustSources(ROOT), params)).toThrow('acting parameter');
  });
  it('refuses removed live gate hops across independent Rust services and desktop method bindings', () => {
    const base = rustSources(ROOT);
    const cases: [string, string, string, string, string][] = [
      [
        'routing.preferences.save',
        'routingPreferencesTrusted',
        'services/routing/preferences.rs',
        'caller.authenticated_host && caller.trusted',
        'true && caller.trusted',
      ],
      ['jobs.list', 'jobsTrusted', 'services/jobs.rs', '!caller.authenticated_host', 'false'],
      [
        'nodes.wake',
        'nodesTrusted',
        'services/nodes/mod.rs',
        'trusted(method, &caller)?;',
        '/* trusted(method, &caller)?; */',
      ],
      [
        'usage.setPacingSchedule',
        'usagePrefsTrusted',
        'services/usage_prefs.rs',
        'if !caller.trusted {',
        'if false {',
      ],
      [
        'machine.stop',
        'machinePowerTrusted',
        'services/machine_power.rs',
        'if !trusted(caller) {',
        'if false {',
      ],
      [
        'remote.tokensList',
        'remotePairingTrusted',
        'services/remote_admin.rs',
        'guard(caller, method, true)?;',
        '/* guard(caller, method, true)?; */',
      ],
      [
        'remote.tailscaleServe',
        'networkTrusted',
        'services/remote_admin.rs',
        'guard(caller, method, false)?;',
        '/* guard(caller, method, false)?; */',
      ],
      [
        'federation.peersConfig',
        'peerConfigTrusted',
        'federation/config.rs',
        'owner(&caller)?;',
        '/* owner(&caller)?; */',
      ],
      [
        'desktop.saveConfig',
        'authenticatedDesktopUser',
        'auth.rs',
        'self.authenticated_host() && desktop_service(method)',
        'true && desktop_service(method)',
      ],
      [
        'files.receiveUpload',
        'authenticatedUploadReceiver',
        'auth.rs',
        'return self.authenticated_host();',
        'return true;',
      ],
      [
        'ui.asset',
        'asset-containment',
        'services/ui_assets.rs',
        'paths::selected_path(root, name)?',
        'root.join(name)',
      ],
      [
        'git.status',
        'canonicalize',
        'services/git.rs',
        'paths::canonicalize(Path::new(requested))?',
        'PathBuf::from(requested)',
      ],
      [
        'plugins.prepareLaunch',
        'authorizeLaunchPreparation',
        'plugins/launch.rs',
        'self.host.check(permit, true).await?;',
        '/* self.host.check(permit, true).await?; */',
      ],
    ];
    for (const [method, guard, file, before, after] of cases) {
      const changed = new Map(base),
        name = 'services/hub-rs/src/' + file,
        text = changed.get(name)!;
      expect(text, before).toContain(before);
      changed.set(name, text.replace(before, after));
      expect(() => guardVerifier(ROOT, changed)(method, guard), method).toThrow();
    }
  });
  it('does not accept a quoted diagnostic as a removed guard call', () => {
    const changed = rustSources(ROOT),
      file = 'services/hub-rs/src/services/nodes/mod.rs';
    changed.set(
      file,
      changed
        .get(file)!
        .replace('trusted(method, &caller)?;', 'let diagnostic = "trusted(method,&caller)?;";'),
    );
    expect(() => guardVerifier(ROOT, changed)('nodes.wake', 'nodesTrusted')).toThrow(
      'missing guard edge',
    );
  });
});
