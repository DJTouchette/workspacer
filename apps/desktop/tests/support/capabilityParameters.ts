import {
  dangerousNames,
  inertMethods,
  methodDecisions,
  parameterDecisions,
  sourceParameterDecisions,
  pathNamespaces,
  pathParameters,
} from './capabilityParameterDecisions';
export {
  dangerousNames,
  inertMethods,
  methodDecisions,
  parameterDecisions,
  sourceParameterDecisions,
  pathNamespaces,
  pathParameters,
};
export const stems = [
  'path',
  'paths',
  'dir',
  'dirs',
  'directory',
  'folder',
  'file',
  'filename',
  'cwd',
  'root',
  'workdir',
  'cmd',
  'command',
  'exec',
  'executable',
  'exe',
  'shell',
  'bin',
  'binary',
  'binaries',
  'launch',
  'launcher',
  'entrypoint',
  'program',
  'script',
  'interpreter',
  'spawn',
  'run',
  'argv',
  'arg',
  'args',
  'argument',
  'arguments',
  'flag',
  'flags',
  'env',
  'environment',
  'url',
  'uri',
  'href',
  'endpoint',
  'webhook',
  'port',
  'socket',
];
export const inertExceptions: Record<string, string> = {};
export function classified(method: string): boolean {
  return [pathParameters, methodDecisions, inertMethods].some((table) =>
    Object.hasOwn(table, method),
  );
}
export function looksPathBearing(method: string): boolean {
  return pathNamespaces.some((prefix) => method.startsWith(prefix));
}
export function missingSpec(method: string): boolean {
  return looksPathBearing(method) && !classified(method);
}
const quote = (s: string): string => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
export function foldName(value: string, keys: Iterable<string>): string | undefined {
  return [...keys].find((key) => new RegExp('^' + quote(key) + '$', 'iu').test(value));
}
export function dangerousKind(value: string, fold = false): string | undefined {
  const key = fold ? foldName(value, Object.keys(dangerousNames)) || '' : value;
  return Object.hasOwn(dangerousNames, key) ? dangerousNames[key] : undefined;
}
export function paramTokens(value: string): string[] {
  const out: string[] = [],
    chars = [...value];
  let current = '';
  const flush = (): void => {
    if (current) {
      out.push(current);
      current = '';
    }
  };
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i];
    if (/[_ .-]|\p{Nd}/u.test(c)) {
      flush();
      continue;
    }
    if (
      /\p{Lu}/u.test(c) &&
      (!i ||
        !/\p{Lu}/u.test(chars[i - 1]) ||
        (i + 1 < chars.length && /\p{Ll}/u.test(chars[i + 1])))
    )
      flush();
    current += [...c.toLowerCase()][0];
  }
  flush();
  return out;
}
export function suspiciousUnknown(value: string): boolean {
  return (
    !!value &&
    !dangerousKind(value, true) &&
    !inertExceptions[value] &&
    paramTokens(value).some((token) => stems.includes(token))
  );
}
export function classifyParam(
  method: string,
  param: string,
  fold = false,
): { status: 'path' | 'decision' | 'unclassified'; kind?: string; reason?: string } {
  if (!Object.hasOwn(pathParameters, method) && !Object.hasOwn(methodDecisions, method))
    return { status: 'unclassified' };
  const scoped = pathParameters[method];
  if (scoped && (fold ? foldName(param, [scoped]) : param === scoped))
    return {
      status: 'path',
      kind: 'path',
      reason: 'Canonical path selection; authenticated paths are ambient, not a filesystem grant.',
    };
  const decisions = { ...parameterDecisions[method], ...sourceParameterDecisions[method] },
    key = fold ? foldName(param, Object.keys(decisions)) : param;
  const decision = key && Object.hasOwn(decisions, key) && decisions[key];
  return decision ? { status: 'decision', ...decision } : { status: 'unclassified' };
}
export function ratchet(observed: number, expected: number): void {
  if (observed !== expected)
    throw Error(`binding ratchet: observed ${observed}, expected ${expected}`);
}
