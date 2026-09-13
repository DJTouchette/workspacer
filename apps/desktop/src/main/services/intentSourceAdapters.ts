import { createHash } from 'node:crypto';
import type {
  IntentSourceConnection,
  IntentSourceSnapshot,
  IntentSourceComment,
} from '../shared/intentSources';

/** Provider contracts checked against primary documentation (2026-09-13):
 * https://developer.atlassian.com/cloud/jira/platform/basic-auth-for-rest-apis/
 * https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issues/
 * https://developer.atlassian.com/cloud/jira/platform/rest/v3/api-group-issue-comments/
 * https://learn.microsoft.com/en-us/rest/api/azure/devops/wit/work-items/get-work-item?view=azure-devops-rest-7.1
 * https://learn.microsoft.com/en-us/rest/api/azure/devops/wit/comments/add-comment?view=azure-devops-rest-7.1
 * https://learn.microsoft.com/en-us/azure/devops/organizations/accounts/use-personal-access-tokens-to-authenticate?view=azure-devops
 * Azure's response example uses commentId while its schema uses id; accept both.
 */

export const SOURCE_RESPONSE_LIMIT = 512 * 1024;
export const SOURCE_TIMEOUT_MS = 8000;
type Receipt = IntentSourceComment['attempts'][number];
export interface IntentSourceAdapter {
  read(
    connection: IntentSourceConnection,
  ): Promise<{ nativeId: string; snapshot: IntentSourceSnapshot }>;
  comment(
    connection: IntentSourceConnection,
    text: string,
  ): Promise<Pick<Receipt, 'status' | 'detail' | 'remoteId'>>;
}
export function sourceText(value: unknown, name: string, max = 128, optional = false): string {
  if (optional && (value === undefined || value === '')) return '';
  if (typeof value !== 'string' || !value.trim() || value.length > max || value.includes('\0'))
    throw new Error(`Invalid ${name}`);
  return value.trim();
}
export function sourceConnection(value: unknown): IntentSourceConnection {
  const v = value as Partial<IntentSourceConnection> | null;
  if (!v || !['manual', 'jira', 'ado'].includes(v.provider || ''))
    throw new Error('Unknown source provider');
  const url = sourceText(v.url, 'source URL', 2048);
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new Error('Invalid source URL');
  }
  if (!['https:', 'http:'].includes(parsed.protocol) || parsed.username || parsed.password)
    throw new Error('Use an HTTP(S) source URL without embedded credentials');
  const connection = {
    provider: v.provider!,
    url: parsed.href,
    credentialEnv: sourceText(v.credentialEnv, 'credential environment name', 128, true),
  };
  if (connection.provider === 'manual') {
    connection.credentialEnv = '';
    return connection;
  }
  if (!/^WORKSPACER_SOURCE_[A-Z0-9_]+$/.test(connection.credentialEnv))
    throw new Error(
      'Credential name must start with WORKSPACER_SOURCE_ and contain uppercase letters, digits or underscores',
    );
  if (parsed.protocol !== 'https:' || parsed.port || parsed.search || parsed.hash)
    throw new Error('Provider URLs must use HTTPS with no port, query or fragment');
  endpoints(connection);
  return connection;
}
/** Never accept a caller-controlled API path or forward authentication across redirects. */
export function endpoints(c: IntentSourceConnection): {
  read: string;
  comment: string;
  nativeId: string;
} {
  const u = new URL(c.url);
  if (u.protocol !== 'https:' || u.username || u.password || u.port || u.search || u.hash)
    throw new Error('Invalid provider URL');
  if (c.provider === 'jira' && /^[a-z0-9][a-z0-9-]*\.atlassian\.net$/.test(u.hostname)) {
    const match = u.pathname.match(/^\/browse\/([A-Z][A-Z0-9_]*-[1-9][0-9]*)\/?$/);
    if (match) {
      const base = `${u.origin}/rest/api/3/issue/${match[1]}`;
      return { read: `${base}?fields=*all`, comment: `${base}/comment`, nativeId: match[1] };
    }
  }
  if (c.provider === 'ado' && u.hostname === 'dev.azure.com') {
    const match = u.pathname.match(
      /^\/([a-zA-Z0-9_-]+)\/([^/]+)\/_workitems\/edit\/([1-9][0-9]*)\/?$/,
    );
    if (match) {
      let project: string;
      try {
        project = decodeURIComponent(match[2]);
      } catch {
        throw new Error('Invalid Azure project');
      }
      if (/[\\/\x00-\x1f]/.test(project) || project === '.' || project === '..')
        throw new Error('Invalid Azure project');
      const base = `${u.origin}/${match[1]}/${encodeURIComponent(project)}/_apis/wit/workitems/${match[3]}`;
      return {
        read: `${base}?api-version=7.1`,
        comment: `${base}/comments?api-version=7.1-preview.4`,
        nativeId: match[3],
      };
    }
  }
  throw new Error(
    'Use a Jira Cloud https://site.atlassian.net/browse/KEY-123 or Azure https://dev.azure.com/org/project/_workitems/edit/123 link',
  );
}
function stable(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(stable).join(',')}]`;
  if (value && typeof value === 'object')
    return `{${Object.entries(value)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([k, v]) => `${JSON.stringify(k)}:${stable(v)}`)
      .join(',')}}`;
  return JSON.stringify(value) ?? 'null';
}
export function sourceSnapshot(
  revision: string,
  title: string,
  content: string,
  fields: Record<string, unknown>,
): IntentSourceSnapshot {
  return {
    revision,
    title,
    content,
    fields,
    fetchedAt: new Date().toISOString(),
    digest: createHash('sha256').update(stable({ revision, title, content, fields })).digest('hex'),
  };
}
function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value))
    throw new Error('Provider returned an invalid record');
  return value as Record<string, unknown>;
}
function adfText(value: unknown, depth = 0): string {
  if (depth > 30 || value === null || value === undefined) return '';
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.map((v) => adfText(v, depth + 1)).join('\n');
  if (typeof value === 'object') {
    const v = value as Record<string, unknown>;
    return typeof v.text === 'string' ? v.text : adfText(v.content, depth + 1);
  }
  return '';
}
class SourceHttpError extends Error {
  constructor(readonly status: number) {
    super(`Provider returned HTTP ${status}`);
  }
}
/** Test injection only; production always uses the host environment and native fetch. */
export function createIntentSourceAdapter(
  fetcher: typeof fetch = fetch,
  env: NodeJS.ProcessEnv = process.env,
): IntentSourceAdapter {
  async function json(c: IntentSourceConnection, write?: string): Promise<Record<string, unknown>> {
    const routes = endpoints(c);
    if (!/^WORKSPACER_SOURCE_[A-Z0-9_]+$/.test(c.credentialEnv))
      throw new Error('Invalid credential environment name');
    const secret = env[c.credentialEnv];
    if (!secret || secret.length > 8192 || /[\r\n]/.test(secret))
      throw new Error(
        'Configure the named credential in the owning host environment and restart the host',
      );
    if (c.provider === 'jira' && !/^[^:]+:.+$/.test(secret))
      throw new Error('Jira credential must contain email:API-token');
    const encoded = Buffer.from(c.provider === 'ado' ? `:${secret}` : secret).toString('base64');
    const signal = AbortSignal.timeout(SOURCE_TIMEOUT_MS);
    try {
      const body =
        write === undefined
          ? undefined
          : JSON.stringify(
              c.provider === 'ado'
                ? { text: write }
                : {
                    body: {
                      type: 'doc',
                      version: 1,
                      content: write.split('\n').map((text) => ({
                        type: 'paragraph',
                        content: text ? [{ type: 'text', text }] : [],
                      })),
                    },
                  },
            );
      const response = await fetcher(write === undefined ? routes.read : routes.comment, {
        method: write === undefined ? 'GET' : 'POST',
        redirect: 'manual',
        signal,
        headers: {
          Authorization: `Basic ${encoded}`,
          Accept: 'application/json',
          ...(body ? { 'Content-Type': 'application/json' } : {}),
        },
        body,
      });
      if (!response.ok) {
        void response.body?.cancel();
        throw new SourceHttpError(response.status);
      }
      if (Number(response.headers.get('content-length')) > SOURCE_RESPONSE_LIMIT) {
        void response.body?.cancel();
        throw new Error('size');
      }
      const reader = response.body?.getReader();
      if (!reader) throw new Error('empty');
      const chunks: Uint8Array[] = [];
      let bytes = 0;
      try {
        while (true) {
          const part = await reader.read();
          if (part.done) break;
          bytes += part.value.length;
          if (bytes > SOURCE_RESPONSE_LIMIT) throw new Error('size');
          chunks.push(part.value);
        }
      } finally {
        void reader.cancel().catch(() => {});
      }
      const raw = Buffer.concat(chunks).toString('utf8');
      // Redact decoded values and keys: JSON escaping can hide a credential from
      // raw-text replacement (quotes, backslashes and unicode escapes).
      const secrets = [
        secret,
        encoded,
        ...(c.provider === 'jira' ? [secret.slice(secret.indexOf(':') + 1)] : []),
      ].filter(Boolean);
      const redactText = (value: string): string =>
        secrets.reduce((result, credential) => result.split(credential).join('[redacted]'), value);
      const redact = (value: unknown, depth = 0): unknown => {
        if (depth > 64) throw new Error('Provider response is too deeply nested');
        if (typeof value === 'string') return redactText(value);
        if (Array.isArray(value)) return value.map((item) => redact(item, depth + 1));
        if (value && typeof value === 'object')
          return Object.fromEntries(
            Object.entries(value).map(([key, item]) => [redactText(key), redact(item, depth + 1)]),
          );
        return value;
      };
      return object(redact(JSON.parse(raw)));
    } catch (error) {
      if (error instanceof SourceHttpError) throw error;
      throw new Error(
        'Provider request failed, timed out, or returned an invalid/oversized response',
      );
    }
  }
  return {
    async read(c) {
      const data = await json(c);
      const fields = object(data.fields);
      const nativeId = endpoints(c).nativeId;
      if (c.provider === 'jira') {
        if (data.key !== nativeId) throw new Error('Provider returned a different issue');
        return {
          nativeId,
          snapshot: sourceSnapshot(
            sourceText(fields.updated, 'Jira revision', 128),
            sourceText(fields.summary, 'Jira title', 4000),
            adfText(fields.description),
            { ...fields, nativeId: data.id, key: data.key },
          ),
        };
      }
      if (String(data.id) !== nativeId || !Number.isInteger(data.rev))
        throw new Error('Provider returned a different work item or invalid revision');
      return {
        nativeId,
        snapshot: sourceSnapshot(
          String(data.rev),
          sourceText(fields['System.Title'], 'Azure title', 4000),
          adfText(fields['System.Description']),
          fields,
        ),
      };
    },
    async comment(c, text) {
      try {
        const data = await json(c, text);
        const remoteId = sourceText(
          String(data.id ?? (c.provider === 'ado' ? data.commentId : undefined) ?? ''),
          'comment receipt',
          128,
        );
        return {
          status: 'accepted',
          detail: 'Provider accepted the comment. Ticket status was not changed.',
          remoteId,
        };
      } catch (error) {
        if (
          error instanceof SourceHttpError &&
          [400, 401, 403, 404, 409, 412, 422, 429].includes(error.status)
        )
          return {
            status: 'failed',
            detail: `Provider refused the comment (HTTP ${error.status}).`,
          };
        return {
          status: 'unknown',
          detail:
            'Publishing outcome is uncertain. Inspect the source comments before taking any further action.',
        };
      }
    },
  };
}
