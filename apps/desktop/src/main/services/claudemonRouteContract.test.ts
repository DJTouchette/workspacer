/** Portable replacement for capspec/claudemoncallers_test.go. No Go runner or
 * generated Go registry is used: both routers and every caller are read directly.
 * The explicit scanner inventory carries floors and reasons; discovery is an
 * independent repository walk. Mutation tests ensure failures are not vacuous. */
import { describe, it, expect } from 'vitest';
import fs from 'fs';
import path from 'path';
import inventory from '../../../tests/support/claudemonCallers.json';
const ROOT = path.resolve(__dirname, '../../../../..');
interface Route {
  server: string;
  pattern: string;
  method: string;
}
interface Caller {
  file: string;
  server: string;
  what: string;
  patterns: string[];
  floor: number;
  suffix: string;
  cutRustTests: boolean;
}
const patterns: Record<string, RegExp> = {
  rustRequestPathRe: /\bpath\s*:\s*(?:&?format!\(\s*)?"(\/[^"\n]*)"/g,
  rustRequestCallRe:
    /\.request\(\s*"(?:GET|POST|PUT|DELETE|PATCH)"\s*,\s*(?:&?format!\(\s*)?"(\/[^"\n]*)"/g,
  rustPathBindingRe: /let\s+(?:mut\s+)?(?:path|root)\s*=\s*(?:format!\(\s*)?"(\/[^"\n]*)"/g,
  rustRootRe: /"(\{root\}\/[^"\n]*)"/g,
  rustExternalRe: /(?:self\.json\(|self\.base\.join\()\s*(?:&?format!\(\s*)?"([^"\n]+)"/g,
  rustSpawnEndpointsRe: /"(\/sessions\/spawn(?:-managed)?)"/g,
  rustEmbeddedSuffixRe: /session_path\(&id,\s*"([^"]+)"/g,
  rustEmbeddedSessionsRe: /Command::Sessions\s*=>\s*\("([^"]+)"/g,
  rustPathLiteralRe: /"(\/[^"\n]*)"/g,
  rustFormatBaseRe: /"\{\}(\/[^"\n]*)"/g,
  rustApiBaseRe: /\{api_base\}(\/[^"\n\\]*)/g,
  rustHookCurlRe: /127\.0\.0\.1:\{hook_port\}(\/[^\s"\\]*)/g,
  rustWrapperBaseRe: /"ws:\/\/127\.0\.0\.1:7891(\/[^"\n]*)"/g,
  tsClaudemonBaseRe: /\$\{CLAUDEMON_API_URL\}([^`'"]*)/g,
  tsCleanupBaseRe: /\$\{daemonUrl\}([^`'"]*)/g,
  tsApiPortRe: /\$\{API_PORT\}([^`'"]*)/g,
  tsRuntimePortRe: /\$\{PORTS\.claudemonApi\}([^`'"]*)/g,
  tsPostJSONRe: /postJSON\(\s*`([^`]*)`/g,
  goPathLiteralRe: /"(\/[^"\n]*)"/g,
  goBasePlusRe: /\bbase\s*\+\s*"(\/[^"\n]*)"/g,
  goLoopbackURLRe: /127\.0\.0\.1:7891([^\s")\n]*)/g,
};
const sources = new Map<string, string>();
function read(file: string): string {
  if (!sources.has(file)) sources.set(file, fs.readFileSync(path.join(ROOT, file), 'utf8'));
  return sources.get(file)!;
}
const productionCache = new Map<string, string>();
function production(src: string): string {
  const original = src;
  const cached = productionCache.get(original);
  if (cached !== undefined) return cached;
  // Test modules can precede production implementations (routing/sampler.rs).
  // Delete balanced modules, not the entire remainder of a source file.
  const re = /#\[cfg\(test\)\]\s*(?:pub\s+)?mod\s+\w+\s*\{/g;
  let match: RegExpExecArray | null;
  while ((match = re.exec(src))) {
    let i = re.lastIndex,
      depth = 1;
    while (i < src.length && depth) {
      if (src.startsWith('//', i)) {
        const end = src.indexOf('\n', i);
        i = end < 0 ? src.length : end;
        continue;
      }
      if (src.startsWith('/*', i)) {
        let comments = 1;
        i += 2;
        while (i < src.length && comments) {
          if (src.startsWith('/*', i)) {
            comments++;
            i += 2;
          } else if (src.startsWith('*/', i)) {
            comments--;
            i += 2;
          } else i++;
        }
        continue;
      }
      const raw = src[i] === 'r' || src[i] === 'b' ? src.slice(i).match(/^(?:b)?r(#+)?"/) : null;
      if (raw) {
        const end = src.indexOf('"' + (raw[1] || ''), i + raw[0].length);
        if (end < 0) throw new Error('unterminated Rust raw string');
        i = end + 1 + (raw[1]?.length || 0);
        continue;
      }
      if (src[i] === '"') {
        i++;
        while (i < src.length) {
          if (src[i] === '\\') i += 2;
          else if (src[i++] === '"') break;
        }
        continue;
      }
      const char = src[i] === "'" ? src.slice(i).match(/^'(?:\\.|[^'\\])'/) : null;
      if (char) {
        i += char[0].length;
        continue;
      }
      if (src[i] === '{') depth++;
      if (src[i] === '}') depth--;
      i++;
    }
    if (depth) throw new Error('unterminated Rust test module');
    const blank = src.slice(match.index, i).replace(/[^\n]/g, ' ');
    src = src.slice(0, match.index) + blank + src.slice(i);
    re.lastIndex = i;
  }
  const result = src.replace(/^[ \t]*\/\/.*$/gm, '');
  productionCache.set(original, result);
  return result;
}
function normalize(raw: string): string {
  let out = '';
  for (let i = 0; i < raw.length;) {
    if (raw[i] === '{' || raw.slice(i, i + 2) === '${') {
      const end = raw.indexOf('}', i) + 1;
      if (!end || (i !== 0 && raw[i - 1] !== '/') || (end !== raw.length && raw[end] !== '/'))
        break;
      out += ':param';
      i = end;
      continue;
    }
    if (!/[a-zA-Z0-9/\-_.~:?#=&%]/.test(raw[i])) break;
    out += raw[i++];
  }
  out = out.split(/[?#]/)[0];
  return out.startsWith('/') ? (out.length > 1 ? out.replace(/\/$/, '') : out) : '';
}
function goConcat(src: string): string {
  for (let i = 0; i < 8; i++) {
    const next = src.replace(/"([^"\n]*)"\s*\+\s*[^"\n+,]+?\s*\+\s*"([^"\n]*)"/g, '"$1:param$2"');
    if (next === src) break;
    src = next;
  }
  return src.replace(/"([^"\n]*\/)"\s*\+\s*([A-Za-z_][\w.]*(?:\([^()]*\))?)/g, '"$1:param"');
}
function scanCaller(c: Caller, source = read(c.file)): { path: string; line: number }[] {
  if (c.cutRustTests) source = production(source);
  if (c.file.endsWith('.go')) source = goConcat(source);
  const seen = new Map<string, { path: string; line: number }>();
  for (const name of c.patterns) {
    const re = patterns[name];
    if (!re) throw new Error(`Unknown scanner ${name} in ${c.file}`);
    for (const match of source.matchAll(re)) {
      let raw = match[1];
      if (name === 'rustRootRe') raw = raw.replace('{root}', '/sessions/:id');
      if (name === 'rustExternalRe') raw = '/' + raw;
      if (name === 'rustEmbeddedSuffixRe') raw = '/sessions/:id/' + raw;
      const p = normalize(raw + c.suffix);
      if (!p) continue;
      const line = source.slice(0, match.index).split('\n').length;
      seen.set(`${p}@${line}`, { path: p, line });
    }
  }
  return [...seen.values()];
}
function served(routes: Route[], server: string, p: string): Route | undefined {
  const want = p.split('/');
  return routes
    .filter((r) => r.server === server)
    .filter((r) => {
      const have = r.pattern.split('/');
      return (
        have.length === want.length && have.every((s, i) => s.startsWith(':') || s === want[i])
      );
    })
    .sort(
      (a, b) =>
        a.pattern.split('/').filter((s) => s.startsWith(':')).length -
        b.pattern.split('/').filter((s) => s.startsWith(':')).length,
    )[0];
}
function scanRouter(src: string, server: string): Route[] {
  src = production(src);
  const sites = [...src.matchAll(/\.route\s*\(/g)];
  const routes = [
    ...src.matchAll(/\.route\s*\(\s*"([^"]+)"\s*,\s*(?:\w+::)*(get|post|put|delete|patch)\s*\(/g),
  ].map((m) => ({ server, pattern: m[1], method: m[2].toUpperCase() }));
  if (routes.length !== sites.length) throw new Error(`${server}: unparsed route registration`);
  return routes;
}
function walk(dir = ROOT): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.isSymbolicLink()) return [];
    const full = path.join(dir, entry.name);
    if (entry.isDirectory())
      return [
        'node_modules',
        'target',
        'dist',
        'release',
        '.git',
        'vendor',
        'out',
        '.rivet',
      ].includes(entry.name)
        ? []
        : walk(full);
    const rel = path.relative(ROOT, full).split(path.sep).join('/');
    return /\.(rs|ts|tsx|go)$/.test(rel) && !/(_test\.go|\.test\.|\/tests\/)/.test(rel)
      ? [rel]
      : [];
  });
}
const legacyPresent = fs.existsSync(path.join(ROOT, 'services/hub/go.mod'));
const active = (file: string): boolean => !file.startsWith('services/hub/') || legacyPresent;
const callers: Caller[] = inventory.callers.filter((c) => active(c.file));
const nonCallers: Record<string, string> = Object.fromEntries(
  Object.entries(inventory.nonCallers).filter(([file]) => active(file)),
);
const reasons: Record<string, string> = inventory.routeReasons;
const routes: Route[] = JSON.parse(read('contracts/claudemon-routes.json')).routes;
const key = (r: Route): string => `${r.server} ${r.pattern}`;
function callerErrors(list: Caller[], table: Route[]): string[] {
  const errors: string[] = [];
  let total = 0;
  for (const c of list) {
    const found = scanCaller(c);
    total += found.length;
    if (found.length < c.floor)
      errors.push(`${c.file}: ${found.length} paths below floor ${c.floor}`);
    for (const p of found)
      if (!served(table, c.server, p.path)) errors.push(`${c.file}:${p.line} unserved ${p.path}`);
  }
  if (total < 60) errors.push(`caller sweep collapsed to ${total} paths`);
  return errors;
}
function orphanErrors(list: Caller[], table: Route[], declared: Record<string, string>): string[] {
  const called = new Set(
    list.flatMap((c) =>
      scanCaller(c).flatMap((p) => {
        const r = served(table, c.server, p.path);
        return r ? [key(r)] : [];
      }),
    ),
  );
  const errors: string[] = [];
  if (called.size < 20) errors.push(`route reachability collapsed to ${called.size}`);
  for (const r of table) {
    const k = key(r),
      reason = declared[k];
    if (called.has(k) && reason) errors.push(`stale caller-less declaration: ${k}`);
    if (!called.has(k) && (!reason || reason.trim().length < 60))
      errors.push(`route lacks caller or reason: ${k}`);
  }
  for (const k of Object.keys(declared))
    if (!table.some((r) => key(r) === k)) errors.push(`declaration names deleted route: ${k}`);
  return errors;
}
function discoveryErrors(
  files: string[],
  declared: Caller[],
  exemptions: Record<string, string>,
): string[] {
  const errors: string[] = [];
  let found = 0;
  for (const file of files) {
    const source = read(file);
    const marker = inventory.markers.find((m) => source.includes(m));
    if (!marker) continue;
    found++;
    if (!declared.some((c) => c.file === file) && !exemptions[file])
      errors.push(`unclaimed caller ${file} (${marker})`);
  }
  if (found < declared.length)
    errors.push(`discovery matched ${found}, below ${declared.length} declared callers`);
  for (const c of declared)
    if (!fs.existsSync(path.join(ROOT, c.file))) errors.push(`deleted caller ${c.file}`);
  for (const [file, reason] of Object.entries(exemptions)) {
    if (!fs.existsSync(path.join(ROOT, file))) errors.push(`deleted non-caller ${file}`);
    if (reason.trim().length < 40) errors.push(`non-caller lacks real reason ${file}`);
    else if (!inventory.markers.some((m) => read(file).includes(m)))
      errors.push(`stale non-caller ${file}`);
  }
  return errors;
}
describe('claudemon route ownership', { timeout: 60_000 }, () => {
  it('fixture_matches_served_routers', () => {
    const actual = [
      ...scanRouter(read('services/claudemon/src/daemon/api.rs'), 'claudemon-api'),
      ...scanRouter(read('services/claudemon/src/daemon/hook.rs'), 'claudemon-hook'),
    ];
    expect(actual.filter((r) => r.server === 'claudemon-api').length).toBeGreaterThanOrEqual(33);
    expect(actual.filter((r) => r.server === 'claudemon-hook').length).toBeGreaterThanOrEqual(4);
    expect(
      [...actual].sort((a, b) =>
        `${a.server} ${a.pattern} ${a.method}`.localeCompare(
          `${b.server} ${b.pattern} ${b.method}`,
        ),
      ),
    ).toEqual(
      [...routes].sort((a, b) =>
        `${a.server} ${a.pattern} ${a.method}`.localeCompare(
          `${b.server} ${b.pattern} ${b.method}`,
        ),
      ),
    );
    // New daemon modules cannot quietly become a third, unenumerated router.
    for (const file of walk(path.join(ROOT, 'services/claudemon/src/daemon'))) {
      if (
        file.endsWith('/api.rs') ||
        file.endsWith('/hook.rs') ||
        file.endsWith('/routes_contract.rs')
      )
        continue;
      expect(scanRouter(read(file), 'claudemon-api'), file).toEqual([]);
    }
  });
  it('every_caller_path_is_served', () => expect(callerErrors(callers, routes)).toEqual([]));
  it('every_caller_file_is_enumerated', () =>
    expect(discoveryErrors(walk(), callers, nonCallers)).toEqual([]));
  it('every_route_has_caller_or_declared_reason', () =>
    expect(orphanErrors(callers, routes, reasons)).toEqual([]));
  it('normalizes interpolations and Go concatenation without inventing suffix routes', () => {
    expect(normalize('/sessions/${id}/transcript${qs}')).toBe('/sessions/:param/transcript');
    expect(normalize('/providers/{provider}/models?x=1')).toBe('/providers/:param/models');
    expect(normalize('/sessions/{id}/')).toBe('/sessions/:param');
    expect(goConcat('"/sessions/" + id + "/input"')).toBe('"/sessions/:param/input"');
    expect(goConcat('"/sessions/"+id')).toBe('"/sessions/:param"');
    expect(served(routes, 'claudemon-api', '/sessions/spawn')?.pattern).toBe('/sessions/spawn');
    expect(served(routes, 'claudemon-api', '/git/status')).toBeUndefined();
    expect(
      scanRouter(
        '.route(\n"/x", axum::routing::post(f))\n#[cfg(test)] mod tests {.route("/fake", get(f))}',
        'api',
      ),
    ).toEqual([{ server: 'api', pattern: '/x', method: 'POST' }]);
    expect(() => scanRouter('.route(dynamic, get(f))', 'api')).toThrow('unparsed');
    expect(
      scanRouter(
        '#[cfg(test)] mod tests { let x = r#"{ fake }"#; }\n.route("/after-tests", get(f))',
        'api',
      ),
    ).toEqual([{ server: 'api', pattern: '/after-tests', method: 'GET' }]);
  });
  it('mutation battery detects missing routes, callers, floors and stale declarations', () => {
    expect(
      callerErrors(
        callers,
        routes.filter((r) => r.pattern !== '/sessions'),
      ),
    ).not.toEqual([]);
    expect(callerErrors([{ ...callers[0], floor: 9999 }], routes).join()).toContain('floor');
    expect(discoveryErrors(walk(), callers.slice(1), nonCallers).join()).toContain(callers[0].file);
    expect(
      orphanErrors(
        callers,
        [...routes, { server: 'claudemon-api', pattern: '/unclaimed', method: 'GET' }],
        reasons,
      ).join(),
    ).toContain('/unclaimed');
    expect(
      orphanErrors(callers, routes, {
        ...reasons,
        'claudemon-api /sessions':
          'A stale declaration must fail even when it contains a sufficiently long explanatory sentence.',
      }).join(),
    ).toContain('stale');
    expect(
      orphanErrors(callers, routes, {
        ...reasons,
        'claudemon-api /deleted':
          'A deleted declaration must fail even when it contains a sufficiently long explanatory sentence.',
      }).join(),
    ).toContain('deleted');
  });
});
