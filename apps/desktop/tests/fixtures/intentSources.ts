/** Synthetic provider-native contracts; no live account data or credentials. */
export const jiraIssue = {
  id: '10001',
  key: 'TEAM-1',
  fields: {
    summary: 'Export CSV',
    updated: '2026-09-13T12:00:00Z',
    description: {
      type: 'doc',
      version: 1,
      content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Accepted requirement' }] }],
    },
    status: { name: 'In Progress', id: '3' },
    resolution: null,
    assignee: { accountId: 'user-1', displayName: 'Owner' },
    priority: { name: 'High' },
    issuelinks: [{ type: { name: 'Blocks' }, outwardIssue: { key: 'TEAM-2' } }],
    attachment: [
      {
        id: 'a',
        filename: 'design.pdf',
        size: 2048,
        mimeType: 'application/pdf',
        content: 'https://do-not-fetch.invalid/body',
      },
    ],
  },
};
export const azurePr = {
  pullRequestId: 12,
  status: 'active',
  title: 'Export CSV',
  description: 'Accepted requirement',
  repository: {
    id: 'repo-id',
    name: 'repo',
    project: { id: '12345678-1234-1234-1234-123456789abc' },
  },
  sourceRefName: 'refs/heads/export',
  targetRefName: 'refs/heads/main',
  mergeStatus: 'succeeded',
  isDraft: false,
  lastMergeSourceCommit: { commitId: 'abc123' },
  reviewers: [{ id: 'reviewer', displayName: 'Reviewer', vote: 10, isRequired: true }],
};
export const azureWorkItem = {
  id: 12,
  rev: 4,
  fields: {
    'System.Title': 'Export CSV',
    'System.Description': 'Accepted requirement',
    'System.State': 'Active',
    'System.WorkItemType': 'Task',
  },
};
