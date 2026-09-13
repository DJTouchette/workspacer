import { describe, it, expect, vi } from 'vitest';
import {
  createIntentSourceAdapter,
  endpoints,
  sourceConnection,
  SOURCE_RESPONSE_LIMIT,
  SOURCE_TIMEOUT_MS,
} from './intentSourceAdapters';
const jira = {
  provider: 'jira' as const,
  url: 'https://team.atlassian.net/browse/TEAM-1',
  credentialEnv: 'WORKSPACER_SOURCE_TEST',
};
const ado = {
  provider: 'ado' as const,
  url: 'https://dev.azure.com/team/My%20Project/_workitems/edit/12',
  credentialEnv: 'WORKSPACER_SOURCE_TEST',
};
const env = { WORKSPACER_SOURCE_TEST: 'person@example.test:super-secret-token' };
const issue = {
  key: 'TEAM-1',
  id: '10001',
  fields: {
    summary: 'Export',
    updated: '2026-09-13',
    description: {
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'text', text: 'CSV rows' }] }],
    },
    status: { id: '7', name: 'Custom state' },
  },
};
describe('source adapter confinement and HTTP contracts', () => {
  it('redacts JSON-escaped credentials in nested keys and values after decoding', async () => {
    const token = 'quote"slash\\token';
    const secret = `person@example.test:${token}`;
    const encoded = Buffer.from(secret).toString('base64');
    const fields = {
      ...issue.fields,
      nested: [{ [token]: secret, header: `Basic ${encoded}`, token }],
    };
    const fetcher = vi.fn().mockResolvedValue(new Response(JSON.stringify({ ...issue, fields })));
    const result = await createIntentSourceAdapter(fetcher, {
      WORKSPACER_SOURCE_TEST: secret,
    }).read(jira);
    expect(result.snapshot.fields.nested).toEqual([
      { '[redacted]': '[redacted]', header: 'Basic [redacted]', token: '[redacted]' },
    ]);
    expect(JSON.stringify(result)).not.toContain(encoded);
  });
  it('allows only known provider routes and disallows credentials and credential env escape', () => {
    expect(endpoints(jira).read).toBe(
      'https://team.atlassian.net/rest/api/3/issue/TEAM-1?fields=*all',
    );
    expect(endpoints(ado).comment).toContain('/team/My%20Project/_apis/wit/workitems/12/comments?');
    for (const url of [
      'http://team.atlassian.net/browse/TEAM-1',
      'https://evil.test/browse/TEAM-1',
      'https://team.atlassian.net.evil.test/browse/TEAM-1',
      'https://u:p@team.atlassian.net/browse/TEAM-1',
      'https://team.atlassian.net/browse/TEAM-1?target=x',
      'https://team.atlassian.net/rest/api/3/issue/TEAM-1',
      'https://team.atlassian.net:444/browse/TEAM-1',
    ])
      expect(() => sourceConnection({ ...jira, url })).toThrow();
    expect(() => sourceConnection({ ...jira, credentialEnv: 'HOME' })).toThrow();
    expect(() =>
      sourceConnection({ ...ado, url: 'https://dev.azure.com/team/a%2fb/_workitems/edit/12' }),
    ).toThrow();
  });
  it('reads Jira provider-native fields and immutable digest without leaking echoed credentials', async () => {
    const fetcher = vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          ...issue,
          fields: { ...issue.fields, echo: env.WORKSPACER_SOURCE_TEST },
        }),
      ),
    );
    const adapter = createIntentSourceAdapter(fetcher, env);
    const result = await adapter.read(jira);
    expect(result.snapshot.content).toBe('CSV rows');
    expect(result.snapshot.fields.status).toEqual(issue.fields.status);
    expect(JSON.stringify(result)).not.toContain('super-secret-token');
    expect(result.snapshot.fields.echo).toBe('[redacted]');
    expect(fetcher.mock.calls[0][1]).toMatchObject({ redirect: 'manual', method: 'GET' });
  });
  it('uses Azure numeric rev and provider fields, and publishes markdown comments only', async () => {
    const fetcher = vi
      .fn()
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({
            id: 12,
            rev: 8,
            fields: {
              'System.Title': 'Title',
              'System.State': 'Review',
              'System.Description': '<p>Task</p>',
            },
          }),
        ),
      )
      .mockResolvedValueOnce(new Response('{"id":34}'));
    const adapter = createIntentSourceAdapter(fetcher, { WORKSPACER_SOURCE_TEST: 'pat' });
    expect((await adapter.read(ado)).snapshot.revision).toBe('8');
    expect(await adapter.comment(ado, 'Reviewed note')).toMatchObject({
      status: 'accepted',
      remoteId: '34',
    });
    expect(JSON.parse(fetcher.mock.calls[1][1].body)).toEqual({ text: 'Reviewed note' });
  });
  it('publishes exact Jira multiline text as ADF', async () => {
    const fetcher = vi.fn().mockResolvedValue(new Response('{"id":"9"}'));
    await createIntentSourceAdapter(fetcher, env).comment(jira, 'One\n\nThree');
    const sent = JSON.parse(fetcher.mock.calls[0][1].body);
    expect(
      sent.body.content
        .map((v: { content: { text: string }[] }) => v.content[0]?.text ?? '')
        .join('\n'),
    ).toBe('One\n\nThree');
  });
  it('never follows redirects and classifies lost/malformed write receipts as uncertain', async () => {
    for (const response of [
      new Response('', { status: 302, headers: { location: 'https://evil.test' } }),
      new Response('bad'),
      new Response('', { status: 500 }),
    ]) {
      const fetcher = vi.fn().mockResolvedValue(response);
      expect((await createIntentSourceAdapter(fetcher, env).comment(jira, 'hello')).status).toBe(
        'unknown',
      );
      expect(fetcher).toHaveBeenCalledTimes(1);
    }
    const fetcher = vi.fn().mockRejectedValue(new Error(env.WORKSPACER_SOURCE_TEST));
    const receipt = await createIntentSourceAdapter(fetcher, env).comment(jira, 'hello');
    expect(receipt.status).toBe('unknown');
    expect(JSON.stringify(receipt)).not.toContain('super-secret-token');
  });
  it('bounds both declared and streamed response size and rejects wrong issue identity', async () => {
    for (const response of [
      new Response('{}', { headers: { 'content-length': String(SOURCE_RESPONSE_LIMIT + 1) } }),
      new Response(' '.repeat(SOURCE_RESPONSE_LIMIT + 1)),
      new Response(JSON.stringify({ ...issue, key: 'OTHER-1' })),
    ])
      await expect(
        createIntentSourceAdapter(vi.fn().mockResolvedValue(response), env).read(jira),
      ).rejects.toThrow();
  });
  it('sets an aborting timeout for requests and does no I/O without configured credentials', async () => {
    const fetcher = vi.fn().mockResolvedValue(new Response(JSON.stringify(issue)));
    const timeout = vi.spyOn(AbortSignal, 'timeout');
    await createIntentSourceAdapter(fetcher, env).read(jira);
    expect(timeout).toHaveBeenCalledWith(SOURCE_TIMEOUT_MS);
    timeout.mockRestore();
    fetcher.mockClear();
    await expect(createIntentSourceAdapter(fetcher, {}).read(jira)).rejects.toThrow('Configure');
    expect(fetcher).not.toHaveBeenCalled();
  });
});
