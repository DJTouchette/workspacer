import type {
  IntentExternalProjection,
  IntentSourceConnection,
  IntentSourceSnapshot,
} from '../shared/intentSources';
import { adfText, endpoints, sourceSnapshot, sourceText } from './intentSourceAdapters';

export interface SourceSyncResult {
  nativeId: string;
  snapshot?: IntentSourceSnapshot;
  projection?: IntentExternalProjection;
  payload?: Record<string, unknown>;
  etag?: string;
  notModified?: boolean;
  partial?: boolean;
}
type Get = (
  url: string,
  etag?: string,
) => Promise<{ data: Record<string, unknown> | null; etag?: string }>;
const record = (v: unknown): Record<string, unknown> =>
  v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : []);
const label = (v: unknown): string => (typeof v === 'string' ? v.slice(0, 240) : '');

/** GET routes are constructed from validated identity. Provider links are stored, never followed.
 * Collections are recent windows, not an exhaustive mirror: omission never implies deletion. */
export async function synchronizeSource(
  c: IntentSourceConnection,
  get: Get,
  etag?: string,
): Promise<SourceSyncResult> {
  const routes = endpoints(c);
  const root = await get(routes.read, etag);
  if (!root.data) return { nativeId: routes.nativeId, notModified: true, etag: root.etag };
  const data = root.data;
  const fields = record(data.fields);
  const pr = routes.objectType === 'pull-request';
  const base = routes.read.split('?')[0];
  const collections: Record<string, unknown> = {};
  let partial = false;
  async function collection(name: string, url: string, key: string, recent = false) {
    try {
      let page = (await get(url)).data!;
      const total = typeof page.total === 'number' ? page.total : undefined;
      // Jira changelog is oldest-first. Jump to the last bounded window; ignore nextPage URLs.
      if (recent && total && total > 50)
        page = (await get(`${url}&startAt=${Math.max(0, total - 50)}`)).data!;
      const raw = list(page[key]);
      const items = [
        ...new Map(
          raw
            .slice(0, 50)
            .map((item, i) => [String(record(item).id ?? record(item).commitId ?? i), item]),
        ).values(),
      ];
      const truncated =
        raw.length > 50 || (total !== undefined ? total > items.length : items.length >= 50);
      collections[name] = {
        items,
        total: total ?? null,
        truncated,
        startAt: page.startAt ?? 0,
        continuationToken: page.continuationToken ?? null,
      };
      if (truncated) partial = true;
    } catch (error) {
      // Rate limits stop this cycle immediately so no subsequent request bypasses Retry-After.
      if (record(error).status === 429) throw error;
      partial = true;
      collections[name] = {
        unavailable: true,
        detail: 'Collection unavailable; prior items are not deleted.',
      };
    }
  }
  let revision: string, title: string, content: string, state: string;
  let summary: Record<string, unknown>;
  if (c.provider === 'jira') {
    if (data.key !== routes.nativeId) throw new Error('Provider returned a different issue');
    revision = sourceText(fields.updated, 'Jira revision', 128);
    title = sourceText(fields.summary, 'Jira title', 4000);
    content = adfText(fields.description);
    state = label(record(fields.status).name);
    await collection('comments', `${base}/comment?maxResults=50&orderBy=-created`, 'comments');
    await collection('recentChanges', `${base}/changelog?maxResults=50`, 'values', true);
    // Attachment URLs are metadata only. No binary, thumbnail, or linked-body request.
    collections.attachments = list(fields.attachment)
      .slice(0, 50)
      .map((a) => {
        const v = record(a);
        return {
          id: v.id,
          filename: v.filename,
          mimeType: v.mimeType,
          size: v.size,
          created: v.created,
          author: v.author,
        };
      });
    summary = {
      status: fields.status,
      resolution: fields.resolution,
      assignee: fields.assignee,
      priority: fields.priority,
      links: fields.issuelinks,
    };
  } else if (pr) {
    if (String(data.pullRequestId) !== routes.nativeId || !record(data.repository).id)
      throw new Error('Provider returned a different or invalid PR');
    title = sourceText(data.title, 'Azure PR title', 4000);
    content = adfText(data.description);
    state = sourceText(data.status, 'Azure PR state', 128);
    revision = sourceSnapshot('pr', title, content, data).digest;
    await collection('threads', `${base}/threads?api-version=7.1`, 'value');
    await collection('commits', `${base}/commits?$top=50&api-version=7.1`, 'value');
    await collection('statuses', `${base}/statuses?$top=50&api-version=7.1`, 'value');
    const projectId = label(record(record(data.repository).project).id);
    if (/^[a-fA-F0-9-]{36}$/.test(projectId)) {
      const policyBase = base.split('/_apis/')[0];
      const artifact = encodeURIComponent(
        `vstfs:///CodeReview/CodeReviewId/${projectId}/${routes.nativeId}`,
      );
      await collection(
        'validation',
        `${policyBase}/_apis/policy/evaluations?artifactId=${artifact}&$top=50&api-version=7.1`,
        'value',
      );
    } else {
      partial = true;
      collections.validation = { unavailable: true };
    }
    summary = {
      repository: { id: record(data.repository).id, name: record(data.repository).name },
      sourceBranch: data.sourceRefName,
      targetBranch: data.targetRefName,
      isDraft: data.isDraft,
      mergeStatus: data.mergeStatus,
      closedDate: data.closedDate,
      lastMergeCommit: record(data.lastMergeCommit).commitId,
      reviewers: list(data.reviewers)
        .slice(0, 50)
        .map((r) => {
          const v = record(r);
          return { id: v.id, displayName: v.displayName, vote: v.vote, isRequired: v.isRequired };
        }),
    };
  } else {
    if (String(data.id) !== routes.nativeId || !Number.isInteger(data.rev))
      throw new Error('Provider returned a different work item or invalid revision');
    revision = String(data.rev);
    title = sourceText(fields['System.Title'], 'Azure title', 4000);
    content = adfText(fields['System.Description']);
    state = label(fields['System.State']);
    summary = {
      type: fields['System.WorkItemType'],
      assignedTo: fields['System.AssignedTo'],
      reason: fields['System.Reason'],
    };
    await collection(
      'comments',
      `${base}/comments?$top=50&order=desc&includeDeleted=true&api-version=7.1-preview.4`,
      'comments',
    );
  }
  // Bounded native objects are retained verbatim after transport redaction, with explicit coverage.
  const payload = { provider: c.provider, url: c.url, native: data, collections };
  if (Buffer.byteLength(JSON.stringify(payload)) > 2 * 1024 * 1024)
    throw new Error('Provider artifact exceeded size limit');
  return {
    nativeId: routes.nativeId,
    snapshot: sourceSnapshot(
      revision,
      title,
      content,
      pr ? data : { ...fields, nativeId: data.id, key: data.key },
    ),
    projection: {
      objectType: pr ? 'pull-request' : c.provider === 'jira' ? 'issue' : 'work-item',
      nativeId: routes.nativeId,
      url: c.url,
      state,
      revision,
      summary,
    },
    payload,
    etag: root.etag,
    partial,
  };
}
