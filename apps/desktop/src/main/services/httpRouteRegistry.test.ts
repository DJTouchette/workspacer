/** Independent HTTP-plane forcing function. Runtime smoke is complementary:
 * this checks every declared route, its actual bound handler, its guard chain,
 * host-only event twins and the complete dynamic operation set. */
import { describe, it, expect } from 'vitest';
import fs from 'fs';
import path from 'path';
import document from '../../../../../contracts/http-route-registry.json';
import vocabulary from '../../../../../services/hub-rs/assets/hub-vocabulary.json';
import {
  body,
  compact,
  endOf,
  maskRust,
  production,
  sites,
  type Site,
} from '../../../tests/support/rustHttpSource';
const ROOT = path.resolve(__dirname, '../../../../..');
type Disposition =
  'guarded' | 'host-only' | 'public-by-decision' | 'tiered-payload' | 'loopback-confined';
interface Row {
  server: string;
  pattern: string;
  file: string;
  verbs: Record<string, string>;
  proofs: Record<string, string>;
  disposition: Disposition;
  reason: string;
  twin?: string;
  twinKind?: string;
  operations?: Record<string, Disposition>;
}
interface Registry {
  routerOwners: Record<string, string>;
  routes: Row[];
}
const registry = document as unknown as Registry;
const read = (file: string): string => fs.readFileSync(path.join(ROOT, file), 'utf8');
const source = new Map(
  [...Object.keys(registry.routerOwners), 'services/hub-rs/src/server/policy.rs'].map((file) => [
    file,
    production(read(file)),
  ]),
);
const key = (r: { server: string; pattern: string }): string => `${r.server} ${r.pattern}`;
const sorted = (value: Record<string, unknown>): string =>
  JSON.stringify(Object.fromEntries(Object.entries(value).sort()));
function requireProof(ok: unknown, reason: string): asserts ok {
  if (!ok) throw new Error(reason);
}
const fn = (sources: Map<string, string>, file: string, name: string): string =>
  compact(body(sources.get(file) ?? read(file), name));
function chain(sources: Map<string, string>, site: Site, verb: string, names: string[]): string {
  requireProof(site.verbs[verb] === names[0], `handler binding drift ${key(site)} ${verb}`);
  for (let i = 0; i < names.length - 1; i++) {
    const caller = compact(maskRust(body(sources.get(site.file)!, names[i])));
    const target = names[i + 1];
    requireProof(caller.startsWith(`${target}(`), `broken handler chain ${names[i]} -> ${target}`);
    const end = endOf(caller, target.length, '(', ')');
    requireProof(
      caller.slice(end) === '.await',
      `handler ${names[i]} must return its guard delegate, not merely mention/call it`,
    );
  }
  return fn(sources, site.file, names[names.length - 1]);
}
function credential(body: string): void {
  requireProof(
    body.startsWith('if!authorized(&s,&h,&q){returndenied();}'),
    'credential refusal must precede handler effects',
  );
}
function host(body: string): void {
  requireProof(
    /^if!host\(&s,&h,&q\)\{returnhost_denied\(&s,&h,&q,"[^"]+"\);\}/.test(body),
    'host refusal must precede handler effects',
  );
}
function operations(bodyText: string): Record<string, string> {
  const code = maskRust(bodyText);
  const marker = /match\s+operation\.as_str\(\)\s*\{/.exec(code);
  requireProof(marker, 'mutable operation dispatcher changed');
  const open = code.indexOf('{', marker.index),
    end = endOf(code, open);
  const codeBody = code.slice(open + 1, end - 1),
    raw = bodyText.slice(open + 1, end - 1);
  const out: Record<string, string> = {};
  let start = 0,
    depth = 0;
  for (let i = 0; i <= codeBody.length; i++) {
    const c = codeBody[i];
    if ('([{'.includes(c ?? '\0')) depth++;
    if (')]}'.includes(c ?? '\0')) depth--;
    if (i !== codeBody.length && (c !== ',' || depth !== 0)) continue;
    const row = compact(raw.slice(start, i));
    start = i + 1;
    if (!row) continue;
    const arm = /^("[^"]+"|_)=>/.exec(row);
    requireProof(arm, `unparsed mutable arm ${row}`);
    const name = arm[1] === '_' ? '_' : JSON.parse(arm[1]);
    requireProof(!(name in out), `duplicate operation ${name}`);
    out[name] = row.slice(arm[0].length);
  }
  requireProof(
    out._ === 'anyhow::bail!("unsupported plugin operation")',
    'unknown mutable operation must refuse',
  );
  delete out._;
  return out;
}
function proof(
  sources: Map<string, string>,
  site: Site,
  verb: string,
  id: string,
  row: Row,
): Disposition {
  const src = sources.get(site.file)!;
  if (id === 'daemon-policy') {
    requireProof(site.server.startsWith('claudemon-'), 'daemon policy attached to wrong listener');
    const router = compact(maskRust(body(src, 'router_with_host')));
    for (const guard of ['host_guard', 'origin_guard'])
      requireProof(
        new RegExp(`\\.layer\\([^;]{0,160}\\b${guard}\\b`).test(router),
        `${site.server} must apply ${guard}, not merely define it`,
      );
    return 'loopback-confined';
  }
  if (id === 'bus-identity') {
    const b = chain(sources, site, verb, ['bus']);
    requireProof(
      compact(maskRust(body(src, 'bus'))).includes(
        'letconnection=matchstate.hub.connect_authenticated_context(presented(&headers,&query).into(),',
      ),
      'bus must authenticate the presented credential',
    );
    requireProof(
      b.includes('Err(_)=>return(StatusCode::UNAUTHORIZED,"unauthorized").into_response()'),
      'bus must refuse authentication failure',
    );
    requireProof(
      b.indexOf('connect_authenticated_context') < b.indexOf('.on_upgrade('),
      'bus upgraded before authentication',
    );
    requireProof(
      b.includes('!state.policy.host(') &&
        b.includes('!state.policy.origin(') &&
        b.indexOf('StatusCode::FORBIDDEN') < b.indexOf('connect_authenticated_context'),
      'bus Host/Origin refusal must precede authentication and upgrade',
    );
    return 'guarded';
  }
  if (id === 'health-projection') {
    const b = chain(sources, site, verb, ['health']);
    requireProof(
      b.includes(
        'if!authorized(&state,&headers,&query){returnJson(json!({"status":"ok"})).into_response();}',
      ),
      'anonymous health must return only liveness',
    );
    requireProof(
      b.indexOf('if!authorized') < b.indexOf('state.hub.health().await'),
      'health topology read moved before policy split',
    );
    return 'tiered-payload';
  }
  if (id === 'plugin-credential') {
    credential(chain(sources, site, verb, [site.verbs[verb]]));
    return 'guarded';
  }
  if (id === 'plugin-host') {
    host(chain(sources, site, verb, [site.verbs[verb]]));
    return 'host-only';
  }
  if (id === 'plugin-settings-delegate') {
    credential(chain(sources, site, verb, ['settings_post', 'mutate']));
    requireProof(
      fn(sources, site.file, 'settings_post').includes('Path("settings".into())'),
      'settings delegate selected a different operation',
    );
    return 'guarded';
  }
  if (id === 'plugin-operations') {
    const b = chain(sources, site, verb, ['mutate']);
    credential(b);
    requireProof(
      b.includes(
        'ifoperation=="reload"&&!host(&s,&h,&q){returnhost_denied(&s,&h,&q,"plugin reload");}',
      ),
      'reload must retain its stronger host gate',
    );
    requireProof(
      b.indexOf('ifoperation=="reload"') < b.indexOf('s.manager.lock().await'),
      'operation effects precede host gate',
    );
    const actual = Object.fromEntries(
      Object.keys(operations(body(src, 'mutate')))
        .sort()
        .map((op) => [op, op === 'reload' ? 'host-only' : 'guarded']),
    );
    requireProof(
      row.operations && sorted(actual) === sorted(row.operations),
      'mutable operation set or required authority changed',
    );
    return 'tiered-payload';
  }
  if (id === 'plugin-list' || id === 'plugin-examples') {
    const b = chain(sources, site, verb, [id === 'plugin-list' ? 'list' : 'examples']);
    requireProof(
      b.includes('ifauthorized(&s,&h,&q){answer(serde_json::to_value(manifests)') &&
        b.includes('else{Json(json!(manifests.into_iter().map(public).collect::<Vec<_>>()))'),
      'manifest public/credential branches changed',
    );
    const publicBody = fn(sources, site.file, 'public');
    requireProof(
      publicBody.includes('fields.iter().filter_map(') &&
        !publicBody.includes('serde_json::to_value(m)'),
      'public manifest must remain explicit-field projection',
    );
    return 'tiered-payload';
  }
  if (id === 'plugin-ui' || id === 'plugin-ui-delegate') {
    const b = chain(sources, site, verb, id === 'plugin-ui' ? ['ui'] : ['ui_index', 'ui']);
    requireProof(
      b.includes(
        'ifauthorized(&s,&h,&query)||own{s.manager.lock().await.settings(&id).ok()}else{None}',
      ),
      'UI settings must remain credential/own-plugin gated',
    );
    requireProof(
      b.includes('letmutown=false;') &&
        b.includes(
          'hub.plugin_token_matches(token.to_owned(),id.clone()).await.unwrap_or(false)',
        ) &&
        b.includes('own=true;break;'),
      'UI own-plugin credential must bind requested plugin through the live authority registry',
    );
    requireProof(
      b.includes('.ui_file(&id,&path)'),
      'UI must resolve assets through semantic plugin containment',
    );
    return 'tiered-payload';
  }
  if (id === 'operator-page') {
    const b = chain(sources, site, verb, ['remote']);
    requireProof(
      b.startsWith('if!state.auth.operator(') &&
        b.includes('return(StatusCode::UNAUTHORIZED,"unauthorized").into_response();'),
      'remote entry operator gate removed',
    );
    return 'guarded';
  }
  if (id === 'app-entry-tier') {
    const b = chain(sources, site, verb, [site.verbs[verb], 'serve_app']);
    requireProof(
      b.includes('letentry=path=="index.html";') &&
        b.includes('ifentry&&!state.auth.operator(') &&
        b.includes('StatusCode::UNAUTHORIZED'),
      'app entry must retain conditional operator gate',
    );
    requireProof(
      b.indexOf('StatusCode::UNAUTHORIZED') < b.indexOf('std::fs::File::open('),
      'app reads precede entry gate',
    );
    requireProof(
      b.includes('target.starts_with(&root)') &&
        b.includes('entry||target!=root.join("index.html")'),
      'public app asset aliases must not expose guarded index',
    );
    return 'tiered-payload';
  }
  if (id === 'app-redirect') {
    const b = chain(sources, site, verb, ['app_redirect']);
    requireProof(
      b.includes('StatusCode::PERMANENT_REDIRECT') &&
        !b.includes('std::fs::') &&
        !b.includes('token'),
      'redirect must remain a stateless location response',
    );
    return 'public-by-decision';
  }
  // /m-next: a redirect to its scope root, and closures that only delegate to
  // `m_next`, which serves a fixed generated include_bytes! table.
  if (id === 'm-next-redirect') {
    const b = chain(sources, site, verb, ['m_next_redirect']);
    requireProof(
      b.includes('StatusCode::PERMANENT_REDIRECT') &&
        b.includes('"/m-next/{}"') &&
        !b.includes('std::fs::') &&
        !b.includes('token'),
      'm-next redirect must remain a stateless location response',
    );
    return 'public-by-decision';
  }
  if (id === 'm-next-static') {
    requireProof(
      site.verbs[verb] === 'inline',
      'm-next static proof cannot certify a named handler',
    );
    requireProof(
      /^,get\(\|[^|]*\|asyncmove\{m_next\((?:&path)?,&headers\)\}\),?$/.test(
        compact(maskRust(site.expression)),
      ),
      'm-next closure must only delegate to m_next',
    );
    const b = fn(sources, site.file, 'm_next');
    requireProof(
      b.includes('m_next_assets::ASSETS') &&
        !/\b(?:State|spawn|request|token|settings|manager)\b/.test(maskRust(b)) &&
        !b.includes('std::fs::'),
      'm_next must serve only the embedded table',
    );
    const allowed = new Set([
      'position',
      'iter',
      'get_or_init',
      'map',
      'digest',
      'collect',
      'format',
      'starts_with',
      'get',
      'and_then',
      'to_str',
      'ok',
      'is_some_and',
      'split',
      'any',
      'trim',
      'into_response',
      'rsplit',
      'next',
      'unwrap_or',
      'mime',
      'asset',
      'headers_mut',
      'insert',
      'parse',
      'unwrap',
      'Some',
      'new',
      'return',
      'as_str',
    ]);
    for (const call of maskRust(body(sources.get(site.file)!, 'm_next')).matchAll(
      /\b([A-Za-z_]\w*)\s*!?\s*\(/g,
    ))
      requireProof(allowed.has(call[1]), `m_next acquired call ${call[1]}`);
    const table = read('services/hub-rs/src/server/m_next_assets.rs');
    const embedded = [...table.matchAll(/include_bytes!\("([^"]+)"\)/g)].map((m) => m[1]);
    const root = '../../assets/web/m-next/';
    requireProof(
      embedded.length > 0 &&
        embedded.every(
          (p) =>
            p.startsWith(root) &&
            !p.slice(root.length).includes('..') &&
            /^[A-Za-z0-9_./-]+$/.test(p),
        ) &&
        !/\b(?:fs|std::env|State)\b/.test(maskRust(table)),
      'm-next asset table escaped its embedded asset root',
    );
    return 'public-by-decision';
  }
  if (id === 'static-web' || id === 'static-sdk') {
    requireProof(
      site.verbs[verb] === 'inline' || site.verbs[verb] === 'public!',
      'static proof cannot certify a named dynamic handler',
    );
    requireProof(
      site.verbs[verb] === 'public!' || /include_(?:bytes|str)!/.test(site.expression),
      'static response must use embedded bytes',
    );
    requireProof(
      !/\b(?:State|spawn|request|token|settings|manager)\b/.test(maskRust(site.expression)),
      'public static response acquired host data or execution',
    );
    const allowedCalls = new Set([
      'get',
      'asset',
      'include_bytes',
      'include_str',
      'headers_mut',
      'insert',
      'parse',
      'unwrap',
    ]);
    if (site.verbs[verb] !== 'public!')
      for (const call of maskRust(site.expression).matchAll(/\b([A-Za-z_]\w*)\s*!?\s*\(/g))
        requireProof(allowedCalls.has(call[1]), `public static closure acquired call ${call[1]}`);
    if (id === 'static-sdk')
      requireProof(
        compact(site.expression).includes('include_str!("sdk.js")'),
        'SDK bytes must remain the fixed embedded SDK',
      );
    else if (site.verbs[verb] === 'public!')
      requireProof(
        /^\s*"[^"]+"\s*,\s*"[A-Za-z0-9_.-]+"/.test(site.expression),
        'public asset macro must name one embedded basename',
      );
    else
      requireProof(
        /include_bytes!\("\.\.\/\.\.\/assets\/web\/[A-Za-z0-9_.-]+"\)/.test(
          compact(site.expression),
        ),
        'public web response escaped embedded asset root',
      );
    return 'public-by-decision';
  }
  if (id === 'plugin-origin') {
    requireProof(
      site.verbs[verb] === 'inline' &&
        compact(site.expression).includes('Json(json!({"origin":s.plugin_origin}))'),
      'public origin response acquired new fields',
    );
    return 'public-by-decision';
  }
  if (id === 'facade-health') {
    requireProof(
      site.pattern === '/health' && site.server === 'mcp' && site.verbs[verb] === 'inline',
      'facade health proof misbound',
    );
    requireProof(
      !/\b(?:token|store|credentials)\b/.test(maskRust(site.expression)),
      'facade health acquired credential material',
    );
    requireProof(
      site.expression.includes('"service":"workspacer-mcp-facade"'),
      'facade health identity changed',
    );
    return 'public-by-decision';
  }
  if (id === 'facade-authenticate') {
    requireProof(
      site.server === 'mcp' && ['/mcp', '/sse'].includes(site.pattern),
      'facade gate misbound',
    );
    const parent = compact(body(sources.get('services/hub-rs/src/mcp.rs')!, 'serve'));
    requireProof(
      /\.layer\(middleware::from_fn_with_state\(Gate\{[^{}]*\},authenticate,\)\)/.test(
        compact(maskRust(body(sources.get('services/hub-rs/src/mcp.rs')!, 'serve'))),
      ),
      'authenticate must be applied as middleware, not merely mentioned',
    );
    const mount = parent.indexOf('.nest_service("/mcp",service)');
    const merge = parent.indexOf('.merge(legacy.router())', mount);
    const gate = parent.indexOf('authenticate,', merge);
    const health = parent.indexOf('router.merge(health)', gate);
    requireProof(
      mount >= 0 &&
        merge > mount &&
        gate > merge &&
        health > gate &&
        !parent.slice(mount, gate).includes(';'),
      'MCP and legacy SSE must be merged under authenticate before public health',
    );
    const b = fn(sources, 'services/hub-rs/src/mcp.rs', 'authenticate');
    requireProof(
      b.includes(
        'gate.resolve(token)else{return(StatusCode::UNAUTHORIZED,"unauthorized").into_response();}',
      ),
      'facade identity resolution must refuse failure',
    );
    requireProof(
      b.includes('gate.policy.host(') && b.includes('gate.policy.origin('),
      'facade Host/Origin gate removed',
    );
    return 'guarded';
  }
  throw new Error(`unknown route proof ${id}`);
}
function primitiveErrors(sources: Map<string, string>): string[] {
  const errors: string[] = [];
  try {
    const file = 'services/hub-rs/src/plugins/http.rs';
    const h = fn(sources, file, 'host'),
      a = fn(sources, file, 'authorized');
    requireProof(
      h.startsWith('credential(h,q).is_some_and(') &&
        h.includes('credential_eq(&s.host_token,&token)') &&
        h.endsWith('s.policy.origin(h,None)') &&
        !/\b(?:Store|Scope|true)\b/.test(h),
      'host primitive acquired a non-owner path',
    );
    requireProof(
      a.startsWith('if!s.policy.host(h,None)||!s.policy.origin(h,None){returnfalse;}') &&
        a.includes('host(s,h,q)||credential(h,q)') &&
        a.endsWith('.is_some_and(|record|record.scope()==Some(crate::auth::Scope::Operator))') &&
        !/\btrue\b/.test(a),
      'operator primitive must retain socket policy and exact scope',
    );
    const operator = fn(sources, 'services/hub-rs/src/server.rs', 'operator');
    requireProof(
      operator.includes('credential_eq(&self.token,token)') &&
        operator.endsWith(
          '.is_some_and(|record|record.scope()==Some(crate::auth::Scope::Operator))',
        ) &&
        !/\btrue\b/.test(operator),
      'web operator primitive changed',
    );
    const guard = fn(sources, 'services/hub-rs/src/server/policy.rs', 'guard');
    requireProof(
      guard.includes('if!policy.host(') &&
        guard.indexOf('StatusCode::FORBIDDEN') < guard.indexOf('next.run(request).await'),
      'outer Host refusal removed',
    );
  } catch (e) {
    errors.push(String(e));
  }
  return errors;
}
function check(doc: Registry, sources: Map<string, string>): string[] {
  const errors: string[] = primitiveErrors(sources);
  const found: Site[] = [];
  for (const [file, server] of Object.entries(doc.routerOwners)) {
    try {
      found.push(...sites(sources.get(file)!, file, server));
    } catch (e) {
      errors.push(String(e));
    }
  }
  if (found.length < 70) errors.push('route enumeration shrank below70');
  // A new third-party/generated router merge can expose routes without any
  // local .route literal. Keep every mounting edge explicit as well.
  const expectedMerges: Record<string, string[]> = {
    'services/hub-rs/src/server.rs': ['assets', 'plugins'],
    'services/hub-rs/src/mcp.rs': ['legacy.router()', 'health'],
  };
  for (const file of Object.keys(doc.routerOwners)) {
    const src = sources.get(file)!,
      code = maskRust(src),
      merged: string[] = [];
    for (const match of code.matchAll(/\.merge\s*\(/g)) {
      const open = code.indexOf('(', match.index),
        end = endOf(code, open, '(', ')');
      merged.push(compact(src.slice(open + 1, end - 1)));
    }
    if (
      JSON.stringify(merged.slice().sort()) !==
      JSON.stringify((expectedMerges[file] ?? []).slice().sort())
    )
      errors.push(`unreviewed router merge ${file}`);
  }
  const registrations = new Set<string>();
  for (const site of found) {
    if (registrations.has(key(site)))
      errors.push(`duplicate registration ${key(site)}; combine its verb handlers for review`);
    registrations.add(key(site));
  }
  const seen = new Set<string>(),
    dispositions = new Set<string>();
  let hostTwins = 0;
  for (const row of doc.routes) {
    const k = key(row);
    if (seen.has(k)) errors.push(`duplicate classification ${k}`);
    seen.add(k);
    dispositions.add(row.disposition);
    const site = found.find((s) => key(s) === k);
    if (!site) {
      errors.push(`stale route classification ${k}`);
      continue;
    }
    if (site.file !== row.file || sorted(site.verbs) !== sorted(row.verbs))
      errors.push(`registration/handler binding drift ${k}`);
    if (
      sorted(Object.fromEntries(Object.keys(row.verbs).map((k) => [k, true]))) !==
      sorted(Object.fromEntries(Object.keys(row.proofs).map((k) => [k, true])))
    )
      errors.push(`proofs do not cover every method ${k}`);
    if (row.reason.trim().length < (row.disposition === 'guarded' ? 40 : 60))
      errors.push(`missing substantive route reason ${k}`);
    for (const [verb, id] of Object.entries(row.proofs)) {
      try {
        const actual = proof(sources, site, verb, id, row);
        if (actual !== row.disposition)
          errors.push(`guard classification mismatch ${k}: ${actual} vs ${row.disposition}`);
      } catch (e) {
        errors.push(`${k}: ${String(e)}`);
      }
    }
    if (row.twinKind === 'event-topic') {
      const topic = vocabulary.topics.find((t) => t.Pattern === row.twin);
      if (!topic) errors.push(`unresolved event twin ${k}`);
      if (topic?.Disposition === 'host-only') {
        hostTwins++;
        if (!['guarded', 'host-only', 'tiered-payload'].includes(row.disposition))
          errors.push(`host-only event payload exposed by ${k}`);
      }
    } else if (row.twinKind === 'bus-method') {
      if (row.twin !== '*' && !vocabulary.methods.includes(row.twin!))
        errors.push(`unresolved method twin ${k}`);
    } else if (row.twin || row.twinKind) errors.push(`invalid twin link ${k}`);
  }
  for (const site of found)
    if (!seen.has(key(site))) errors.push(`unclassified route ${key(site)}`);
  for (const d of [
    'guarded',
    'host-only',
    'tiered-payload',
    'public-by-decision',
    'loopback-confined',
  ])
    if (!dispositions.has(d)) errors.push(`disposition vanished ${d}`);
  if (hostTwins < 3) errors.push('host-only cross-plane twins collapsed');
  for (const [file, owner] of Object.entries(doc.routerOwners)) {
    if (!sources.has(file)) errors.push(`missing router source ${file}`);
    if (owner === 'hub' && file.endsWith('/server.rs')) {
      const c = compact(sources.get(file)!);
      if (
        !c.includes(
          'router=router.layer(axum::middleware::from_fn_with_state(policy,policy::guard))',
        )
      )
        errors.push('hub outer socket policy removed');
    }
  }
  return errors;
}
function allRustFiles(directory: string): string[] {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((e) => {
    if (e.isSymbolicLink()) return [];
    const full = path.join(directory, e.name);
    return e.isDirectory()
      ? allRustFiles(full)
      : e.name.endsWith('.rs')
        ? [path.relative(ROOT, full).split(path.sep).join('/')]
        : [];
  });
}
function discover(files: Map<string, string>, owners: Record<string, string>): string[] {
  const ignored = new Set<string>(),
    liveImports = new Set<string>();
  const resolve = (file: string, declaration: string, name: string): string[] => {
    const parent = path.posix.dirname(file);
    const explicit = /#\[path\s*=\s*"([^"]+)"\]/.exec(declaration);
    if (explicit) return [path.posix.join(parent, explicit[1])];
    const base = ['mod.rs', 'lib.rs', 'main.rs'].includes(path.posix.basename(file))
      ? parent
      : file.replace(/\.rs$/, '');
    return [
      path.posix.join(base, name.replace(/^r#/, '') + '.rs'),
      path.posix.join(base, name.replace(/^r#/, ''), 'mod.rs'),
    ];
  };
  for (const [file, raw] of files) {
    const src = production(raw);
    let code = maskRust(src);
    const tests = [
      ...code.matchAll(
        /#\[cfg\(test\)\]\s*(?:#\[path\s*=[^\]]*\]\s*)?(?:pub(?:\([^)]*\))?\s+)?mod\s+([^\s;]+)\s*;/g,
      ),
    ];
    for (const declaration of tests.reverse()) {
      const start = declaration.index!,
        end = start + declaration[0].length;
      for (const candidate of resolve(file, src.slice(start, end), declaration[1]))
        ignored.add(candidate);
      code = code.slice(0, start) + code.slice(start, end).replace(/[^\n]/g, ' ') + code.slice(end);
    }
    // A test import cannot hide a file that is ALSO imported by production.
    // Read attributes from original text only after structural code matched;
    // comments and strings therefore cannot manufacture an exemption.
    for (const declaration of code.matchAll(
      /(?:#\[path\s*=[^\]]*\]\s*)?\b(?:pub(?:\([^)]*\))?\s+)?mod\s+([^\s;]+)\s*;/g,
    )) {
      const start = declaration.index!,
        end = start + declaration[0].length;
      for (const candidate of resolve(file, src.slice(start, end), declaration[1]))
        liveImports.add(candidate);
    }
  }
  const unknown: string[] = [];
  for (const [file, src] of files) {
    if (ignored.has(file) && !liveImports.has(file)) continue;
    const code = maskRust(production(src));
    if (
      /\.(?:route|route_service|nest|nest_service|fallback|fallback_service)\s*\(/.test(code) &&
      !owners[file]
    )
      unknown.push(file);
  }
  return unknown;
}
describe('HTTP registry source and cross-plane policy', { timeout: 90_000 }, () => {
  it('http_registry_matches_actual_bindings', () => expect(check(registry, source)).toEqual([]));
  it('discovers new route-bearing production modules instead of trusting the router list', () => {
    const files = new Map(
      [
        ...allRustFiles(path.join(ROOT, 'services/hub-rs/src')),
        ...allRustFiles(path.join(ROOT, 'services/claudemon/src/daemon')),
      ].map((file) => [file, read(file)]),
    );
    expect(discover(files, registry.routerOwners)).toEqual([]);
    files.set(
      'services/hub-rs/src/unlisted.rs',
      'fn router(){Router::new().route("/hidden",get(secret))}',
    );
    expect(discover(files, registry.routerOwners)).toContain('services/hub-rs/src/unlisted.rs');
    files.delete('services/hub-rs/src/unlisted.rs');
    const parent = 'services/hub-rs/src/fixture.rs',
      child = 'services/hub-rs/src/fixture/tests.rs';
    files.set(parent, '#[cfg(test)] mod tests;');
    files.set(child, 'fn fixture(){Router::new().route("/test",get(dummy))}');
    expect(discover(files, registry.routerOwners)).not.toContain(child);
    files.set(parent, 'mod tests;');
    expect(discover(files, registry.routerOwners)).toContain(child);
    files.set(parent, '// #[cfg(test)] mod tests;');
    expect(discover(files, registry.routerOwners)).toContain(child);
    files.set(parent, '#[cfg(test)] mod tests;');
    files.set(
      'services/hub-rs/src/fixture_import.rs',
      '#[path = "fixture/tests.rs"] mod real_router;',
    );
    expect(discover(files, registry.routerOwners)).toContain(child);
  });
  it('holds credential primitives to operator versus actual host authority', () =>
    expect(primitiveErrors(source)).toEqual([]));
  it('rejects mutations of routes, classifiers, guards, operation closure and live confinement layers', () => {
    expect(check(registry, source)).toEqual([]);
    const reject = (label: string, edit: (doc: Registry, sources: Map<string, string>) => void) => {
      const doc = structuredClone(registry),
        sources = new Map(source);
      edit(doc, sources);
      expect(check(doc, sources), label).not.toEqual([]);
    };
    reject('new unclassified route', (_, s) =>
      s.set(
        'services/hub-rs/src/server.rs',
        s
          .get('services/hub-rs/src/server.rs')!
          .replace(
            '.route("/health", get(health))',
            '.route("/unclassified", get(health)).route("/health", get(health))',
          ),
      ),
    );
    reject('new method on an existing route', (_, s) =>
      s.set(
        'services/hub-rs/src/server.rs',
        s
          .get('services/hub-rs/src/server.rs')!
          .replace(
            '.route("/health", get(health))',
            '.route("/health", post(health)).route("/health", get(health))',
          ),
      ),
    );
    reject('unreviewed router mount', (_, s) =>
      s.set(
        'services/hub-rs/src/server.rs',
        s
          .get('services/hub-rs/src/server.rs')!
          .replace('.merge(assets)', '.merge(assets).merge(unclassified_router)'),
      ),
    );
    reject('stale row', (d) => d.routes.push({ ...d.routes[0], pattern: '/deleted' }));
    reject('missing row', (d) => {
      d.routes.shift();
    });
    reject('duplicate row', (d) => d.routes.push(d.routes[0]));
    reject('host downgrade', (d) => {
      d.routes.find((r) => r.pattern === '/plugins/install')!.disposition = 'guarded';
    });
    reject('plugin token authority bypass', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s
          .get('services/hub-rs/src/plugins/http.rs')!
          .replace(
            'plugin_token_matches(token.to_owned(), id.clone())',
            'plugin_token_matches(token.to_owned(), "other".into())',
          ),
      ),
    );
    reject('plugin authority lookup fails open', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s
          .get('services/hub-rs/src/plugins/http.rs')!
          .replace('.unwrap_or(false)', '.unwrap_or(true)'),
      ),
    );
    reject('credential removal', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s
          .get('services/hub-rs/src/plugins/http.rs')!
          .replaceAll('if !authorized(&s, &h, &q) {', 'if false {'),
      ),
    );
    reject('host gate replacement', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s
          .get('services/hub-rs/src/plugins/http.rs')!
          .replaceAll('if !host(&s, &h, &q) {', 'if !authorized(&s, &h, &q) {'),
      ),
    );
    reject('mutable new operation', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s
          .get('services/hub-rs/src/plugins/http.rs')!
          .replace(
            'match operation.as_str(){',
            'match operation.as_str(){"newDangerousOperation"=>Ok(json!({})), ',
          ),
      ),
    );
    reject('operation literal whitespace changes authority', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s.get('services/hub-rs/src/plugins/http.rs')!.replace('"reload"=>', '"re load"=>'),
      ),
    );
    reject('mutable default succeeds', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s
          .get('services/hub-rs/src/plugins/http.rs')!
          .replace('anyhow::bail!("unsupported plugin operation")', 'Ok(json!({}))'),
      ),
    );
    reject('host event bytes public', (d) => {
      d.routes.find((r) => r.pattern === '/plugins')!.disposition = 'public-by-decision';
    });
    reject('stale event twin', (d) => {
      d.routes.find((r) => r.pattern === '/plugins')!.twin = 'no.such.topic';
    });
    reject('public manifest widened', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s
          .get('services/hub-rs/src/plugins/http.rs')!
          .replaceAll('.map(public)', '.map(serde_json::to_value)'),
      ),
    );
    reject('host helper admits operator', (_, s) => {
      const f = 'services/hub-rs/src/plugins/http.rs',
        b = body(s.get(f)!, 'host');
      s.set(f, s.get(f)!.replace(b, 'true'));
    });
    reject('operator helper widens scope', (_, s) =>
      s.set(
        'services/hub-rs/src/plugins/http.rs',
        s.get('services/hub-rs/src/plugins/http.rs')!.replaceAll('Scope::Operator', 'Scope::View'),
      ),
    );
    reject('outer Host guard disappears', (_, s) => {
      const f = 'services/hub-rs/src/server/policy.rs',
        b = body(s.get(f)!, 'guard');
      s.set(f, s.get(f)!.replace(b, 'next.run(request).await'));
    });
    reject('public closure grows a side effect', (_, s) =>
      s.set(
        'services/hub-rs/src/server/web.rs',
        s
          .get('services/hub-rs/src/server/web.rs')!
          .replace(
            'let mut response = asset(',
            'let leaked = read_secret(); let mut response = asset(',
          ),
      ),
    );
    reject('quoted guard reference does not apply a layer', (_, s) => {
      const file = 'services/claudemon/src/daemon/api.rs',
        original = s.get(file)!,
        router = body(original, 'router_with_host');
      const changed =
        'let decoy = r#".layer(middleware::from_fn(host_guard))"#;' +
        router.replaceAll('host_guard', 'unrelated_handler');
      s.set(file, original.replace(router, changed));
    });
    reject('quoted delegation does not guard the route', (_, s) => {
      const file = 'services/hub-rs/src/plugins/http.rs',
        original = s.get(file)!,
        wrapper = body(original, 'settings_post');
      s.set(file, original.replace(wrapper, 'let decoy = "mutate("; denied()'));
    });
    reject('MCP gate removed', (_, s) =>
      s.set(
        'services/hub-rs/src/mcp.rs',
        s
          .get('services/hub-rs/src/mcp.rs')!
          .replace('            authenticate,', '            unrelated_handler,'),
      ),
    );
    for (const file of [
      'services/claudemon/src/daemon/api.rs',
      'services/claudemon/src/daemon/hook.rs',
    ])
      for (const guard of ['host_guard', 'origin_guard'])
        reject(`${file} ${guard} unapplied`, (_, s) => {
          const src = s.get(file)!,
            b = body(src, 'router_with_host');
          s.set(file, src.replace(b, b.replaceAll(guard, 'unrelated_handler')));
        });
  });
  it('keeps the served fixture and all four caller ownership guards linked', () => {
    const fixture = JSON.parse(read('contracts/claudemon-routes.json'));
    const registryRoutes = registry.routes
      .filter((r) => r.server.startsWith('claudemon-'))
      .flatMap((r) => Object.keys(r.verbs).map((method) => `${r.server} ${method} ${r.pattern}`))
      .sort();
    expect(registryRoutes).toEqual(
      fixture.routes
        .map(
          (r: { server: string; method: string; pattern: string }) =>
            `${r.server} ${r.method} ${r.pattern}`,
        )
        .sort(),
    );
    const callerFile = 'apps/desktop/src/main/services/claudemonRouteContract.test.ts';
    expect(fixture.vocabulary.blocks.routes.loaders).toContain(
      `${callerFile}::fixture_matches_served_routers`,
    );
    for (const name of [
      'fixture_matches_served_routers',
      'every_caller_path_is_served',
      'every_caller_file_is_enumerated',
      'every_route_has_caller_or_declared_reason',
    ])
      expect(read(callerFile)).toContain(name);
  });
  it('source parser ignores comments and quoted decoys but retains post-test production', () => {
    const text =
      '// .route("/comment", get(f))\n#[cfg(test)] mod tests {let x=r#"{ }"#;}\nRouter::new().route("/real", get(real).post(write))';
    expect(
      sites(production(text), 'fixture', 'hub').map(({ pattern, verbs }) => ({ pattern, verbs })),
    ).toEqual([{ pattern: '/real', verbs: { GET: 'real', POST: 'write' } }]);
    expect(() => sites('Router::new().route(variable,get(f))', 'fixture', 'hub')).toThrow(
      'dynamic',
    );
    expect(() => sites('Router::new().fallback(handler)', 'fixture', 'hub')).toThrow(
      'unclassified',
    );
    expect(() =>
      sites(
        'Router::new().route("/x", get(read).on(MethodFilter::POST, mutate))',
        'fixture',
        'hub',
      ),
    ).toThrow('unparsed');
    expect(() =>
      sites('Router::new().route("/x", get(read).get(other))', 'fixture', 'hub'),
    ).toThrow('duplicate');
    expect(() => body('fn f(){} fn f(){}', 'f')).toThrow('one definition');
    expect(compact(' operation == "re load" ')).toBe('operation=="re load"');
    expect(compact(' operation == r#"re load"# ')).toBe('operation==r#"re load"#');
  });
});
