import React from 'react';
import { beforeEach, expect, it, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import IntentArtifacts, { staticArtifactHtml } from '../../src/components/IntentArtifacts';
import type { IntentArtifact, IntentArtifactResponse } from '../../../main/shared/intentArtifacts';
import type { IntentWorkspace } from '../../../main/shared/intentWorkspace';
const workspace: IntentWorkspace = {
  id: 'work',
  title: 'Export',
  projectRoot: '/project',
  revision: 1,
  outcome: '',
  constraints: '',
  successCriteria: 'Works offline',
  sourceUrl: '',
  status: 'active',
  createdAt: '',
  updatedAt: '',
};
const image: IntentArtifact = {
  id: 'image',
  workspaceId: 'work',
  intentRevision: 1,
  title: 'Export screenshot',
  kind: 'image',
  mimeType: 'image/png',
  bytes: 8,
  sha256: 'a'.repeat(64),
  contentId: 'stored',
  createdAt: '2026-09-13T00:00:00Z',
  author: 'user',
};
const request = vi.fn();
const openUrl = vi.fn();
let listing: Extract<IntentArtifactResponse, { action: 'artifacts' }>;
beforeEach(() => {
  listing = {
    action: 'artifacts',
    artifacts: [image],
    annotations: [],
    demonstrations: [],
    groups: [],
    selections: [],
  };
  openUrl.mockReset();
  request.mockReset().mockImplementation(async (input) => {
    if (input.action === 'artifacts') return structuredClone(listing);
    if (input.action === 'executions') return { action: 'executions', executions: [], links: [] };
    if (input.action === 'readArtifact')
      return {
        action: 'readArtifact',
        artifact: listing.artifacts.find((a) => a.id === input.artifactId),
        dataBase64: btoa('preview'),
      };
    if (input.action === 'annotateArtifact') {
      const annotation = {
        ...input,
        id: input.annotationId,
        author: 'user',
        createdAt: '2026-09-13T00:00:00Z',
      };
      listing.annotations.push(annotation);
      return { action: input.action, annotation };
    }
    if (input.action === 'createDemonstration') {
      const demonstration = {
        id: input.demonstrationId,
        title: input.title,
        steps: input.steps,
        intentRevision: 1,
      };
      listing.demonstrations.push(demonstration as never);
      return { action: input.action, demonstration };
    }
    if (input.action === 'addArtifact') {
      const artifact = {
        ...image,
        id: input.artifactId,
        title: input.title,
        kind: input.url ? 'url' : 'text',
        url: input.url,
      };
      listing.artifacts.push(artifact as IntentArtifact);
      return { action: input.action, artifact };
    }
    if (input.action === 'createAlternativeGroup') {
      const group = { ...input, id: input.groupId, intentRevision: 1 };
      listing.groups.push(group);
      return { action: input.action, group };
    }
    if (input.action === 'selectAlternative') {
      const selection = { ...input, id: input.selectionId, createdAt: '2026-09-13T00:00:00Z' };
      listing.selections.unshift(selection);
      return { action: input.action, selection };
    }
    throw new Error('Unexpected request');
  });
  Object.assign(window.electronAPI, { intentWorkspaceRequest: request });
});
const view = (visible = true, disabled = false) => (
  <IntentArtifacts
    workspace={workspace}
    visible={visible}
    disabled={disabled}
    onOpenUrl={openUrl}
  />
);
async function selectImage() {
  await screen.findByRole('option', { name: /Export screenshot · image/ });
  fireEvent.change(screen.getByLabelText('Saved artifact'), { target: { value: 'image' } });
  await screen.findByRole('img', { name: 'Export screenshot' });
}
it('requires explicit annotation mode and pins normalized points to the displayed artifact digest', async () => {
  render(view());
  await selectImage();
  expect(screen.queryByLabelText('Annotation note')).not.toBeInTheDocument();
  const preview = screen.getByRole('img', { name: 'Export screenshot' });
  vi.spyOn(preview, 'getBoundingClientRect').mockReturnValue({
    left: 20,
    top: 10,
    width: 200,
    height: 100,
  } as DOMRect);
  fireEvent.click(preview, { clientX: 70, clientY: 85 });
  expect(screen.queryByLabelText('Selected annotation point')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Annotate', exact: true }));
  fireEvent.click(preview, { clientX: 70, clientY: 85 });
  fireEvent.change(screen.getByLabelText('Annotation note'), {
    target: { value: 'Make the export action clearer.' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Save annotation' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'annotateArtifact',
        artifactId: 'image',
        artifactSha256: image.sha256,
        point: { x: 0.25, y: 0.75 },
        expectedRevision: 1,
      }),
    ),
  );
  expect(await screen.findByText('Make the export action clearer.')).toBeInTheDocument();
});
it('preserves an unsaved artifact draft when hidden and after a save error; URL opening is deliberate', async () => {
  const rendered = render(view());
  fireEvent.click(screen.getByText('Add an artifact or version'));
  fireEvent.change(screen.getByLabelText('Artifact title'), { target: { value: 'Preview link' } });
  fireEvent.change(screen.getByLabelText('Or URL reference'), {
    target: { value: 'https://example.com/preview' },
  });
  rendered.rerender(view(false));
  rendered.rerender(view());
  expect(screen.getByLabelText('Artifact title')).toHaveValue('Preview link');
  request.mockImplementationOnce(async () => {
    throw new Error('Storage unavailable');
  });
  // Wait for the tab reload before arranging the mutation failure.
  await screen.findByLabelText('Saved artifact');
  fireEvent.click(screen.getByRole('button', { name: 'Save artifact version' }));
  await waitFor(() =>
    expect(request.mock.calls.some(([i]) => i.action === 'addArtifact')).toBe(true),
  );
  expect(await screen.findByRole('alert')).toHaveTextContent('Storage unavailable');
  expect(screen.getByLabelText('Artifact title')).toHaveValue('Preview link');
  expect(openUrl).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: 'Save artifact version' }));
  const open = await screen.findByRole('button', { name: 'Open reference' });
  expect(openUrl).not.toHaveBeenCalled();
  fireEvent.click(open);
  expect(openUrl).toHaveBeenCalledExactlyOnceWith('https://example.com/preview');
});
it('records an ordered screenshot demonstration with explicit captions', async () => {
  render(view());
  await selectImage();
  fireEvent.click(screen.getByText('Create a demonstration'));
  fireEvent.change(screen.getByLabelText('Demonstration title'), {
    target: { value: 'Export flow' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Add selected screenshot as next step' }));
  fireEvent.change(screen.getByLabelText('Step 1 caption'), {
    target: { value: 'Choose Export and inspect the CSV.' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Save demonstration' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'createDemonstration',
        steps: [{ artifactId: 'image', caption: 'Choose Export and inspect the CSV.' }],
      }),
    ),
  );
  expect(await screen.findByText('Export flow · demonstration · intent 1')).toBeInTheDocument();
});

it('uploads bounded file bytes without exposing a client filesystem path', async () => {
  render(view());
  fireEvent.click(screen.getByText('Add an artifact or version'));
  const input = screen.getByLabelText('Upload file (up to 512 KiB)');
  fireEvent.change(input, {
    target: {
      files: [new File([new Uint8Array(512 * 1024 + 1)], 'large.png', { type: 'image/png' })],
    },
  });
  expect(await screen.findByRole('alert')).toHaveTextContent('exceeds 512 KiB');
  expect(request.mock.calls.some(([i]) => i.action === 'addArtifact')).toBe(false);
  fireEvent.change(input, {
    target: { files: [new File(['A plain note'], 'note.md', { type: 'text/markdown' })] },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Save artifact version' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'addArtifact',
        title: 'note.md',
        mimeType: 'text/markdown',
        dataBase64: btoa('A plain note'),
      }),
    ),
  );
  const sent = request.mock.calls.find(([i]) => i.action === 'addArtifact')![0];
  expect(sent.path).toBeUndefined();
  expect(sent.cwd).toBeUndefined();
  expect(openUrl).not.toHaveBeenCalled();
});
it('renders HTML under an empty sandbox and strips active content and navigation', async () => {
  const unsafe =
    '<script>window.parent.alert(1)</script><meta http-equiv="refresh" content="0;url=https://example.com"><a href="https://example.com">Go</a><iframe src="https://example.com"></iframe><form action="https://example.com"><input></form><p onclick="alert(1)">Static text</p>';
  const safe = staticArtifactHtml(unsafe);
  expect(safe).not.toMatch(/<script|<iframe|<form|onclick|http-equiv="refresh"|href=/);
  expect(safe).toContain("default-src 'none'");
  listing.artifacts = [
    { ...image, id: 'html', title: 'Mockup', kind: 'html', mimeType: 'text/html' },
  ];
  request.mockImplementation(async (input) => {
    if (input.action === 'artifacts') return listing;
    if (input.action === 'executions') return { action: 'executions', executions: [] };
    return { action: 'readArtifact', artifact: listing.artifacts[0], dataBase64: btoa(unsafe) };
  });
  render(view());
  await screen.findByRole('option', { name: /Mockup · html/ });
  fireEvent.change(screen.getByLabelText('Saved artifact'), { target: { value: 'html' } });
  const frame = await screen.findByTitle('Static preview of Mockup');
  expect(frame).toHaveAttribute('sandbox', '');
  expect(frame).toHaveAttribute('referrerpolicy', 'no-referrer');
  expect(frame.getAttribute('srcdoc')).toContain('Static text');
});
it('records comparison purpose and budget, then requires a reason before selecting', async () => {
  render(view());
  await screen.findByLabelText('Saved artifact');
  fireEvent.click(screen.getByText('Compare alternatives'));
  fireEvent.change(screen.getByLabelText('Comparison title'), {
    target: { value: 'Export approach' },
  });
  fireEvent.change(screen.getByLabelText('Purpose'), {
    target: { value: 'Choose lower memory use' },
  });
  for (let i = 1; i <= 2; i++) {
    fireEvent.change(screen.getByLabelText(`Alternative ${i} name`), {
      target: { value: i === 1 ? 'Stream' : 'Batch' },
    });
    fireEvent.change(screen.getByLabelText(`Alternative ${i} hypothesis`), {
      target: { value: i === 1 ? 'Lower memory' : 'Simpler checks' },
    });
  }
  fireEvent.click(screen.getByRole('button', { name: 'Save comparison' }));
  const summary = await screen.findByText('Export approach · alternatives · intent 1');
  fireEvent.click(summary);
  expect(screen.getByRole('button', { name: 'Record selection' })).toBeDisabled();
  const group = listing.groups[0];
  fireEvent.change(screen.getByLabelText('Choose alternative'), {
    target: { value: group.alternatives[0].id },
  });
  fireEvent.change(screen.getByLabelText('Selection reason'), {
    target: { value: 'The memory constraint decides it.' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Record selection' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'selectAlternative',
        groupId: group.id,
        alternativeId: group.alternatives[0].id,
        reason: 'The memory constraint decides it.',
      }),
    ),
  );
  expect(await screen.findByText('Stream · selected')).toBeInTheDocument();
});

it('retains annotation drafts per version and supports keyboard point placement', async () => {
  listing.artifacts.push({
    ...image,
    id: 'second',
    title: 'Second screenshot',
    sha256: 'b'.repeat(64),
  });
  const rendered = render(view());
  await selectImage();
  fireEvent.click(screen.getByRole('button', { name: 'Annotate', exact: true }));
  fireEvent.change(screen.getByLabelText('Annotation note'), {
    target: { value: 'First screenshot note' },
  });
  const canvas = screen.getByRole('group', { name: 'Image annotation position' });
  expect(canvas).toHaveAttribute('tabindex', '0');
  fireEvent.keyDown(canvas, { key: 'Enter' });
  fireEvent.keyDown(canvas, { key: 'ArrowRight' });
  expect(screen.getByText('Annotation point: 51% across, 50% down.')).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('Saved artifact'), { target: { value: 'second' } });
  await screen.findByRole('img', { name: 'Second screenshot' });
  fireEvent.click(screen.getByRole('button', { name: 'Annotate', exact: true }));
  expect(screen.getByLabelText('Annotation note')).toHaveValue('');
  fireEvent.change(screen.getByLabelText('Annotation note'), {
    target: { value: 'Second screenshot note' },
  });
  rendered.rerender(view(false));
  rendered.rerender(view());
  fireEvent.change(screen.getByLabelText('Saved artifact'), { target: { value: 'image' } });
  await screen.findByRole('img', { name: 'Export screenshot' });
  fireEvent.click(screen.getByRole('button', { name: 'Annotate', exact: true }));
  expect(screen.getByLabelText('Annotation note')).toHaveValue('First screenshot note');
  expect(screen.getByText('Annotation point: 51% across, 50% down.')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: 'Save annotation' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'annotateArtifact',
        artifactId: 'image',
        artifactSha256: image.sha256,
        point: { x: 0.51, y: 0.5 },
        text: 'First screenshot note',
      }),
    ),
  );
});

it('retries an unchanged artifact save with the same identity after an ambiguous response', async () => {
  const normal = request.getMockImplementation()!;
  let lost = true;
  request.mockImplementation(async (input) => {
    if (input.action === 'addArtifact' && lost) {
      lost = false;
      throw new Error('Connection lost after save');
    }
    return normal(input);
  });
  render(view());
  await screen.findByLabelText('Saved artifact');
  fireEvent.click(screen.getByText('Add an artifact or version'));
  fireEvent.change(screen.getByLabelText('Artifact title'), {
    target: { value: 'Saved reference' },
  });
  fireEvent.change(screen.getByLabelText('Or URL reference'), {
    target: { value: 'https://example.com/reference' },
  });
  fireEvent.click(screen.getByRole('button', { name: 'Save artifact version' }));
  await screen.findByRole('alert');
  fireEvent.click(screen.getByRole('button', { name: 'Save artifact version' }));
  await screen.findByRole('button', { name: 'Open reference' });
  const attempts = request.mock.calls.filter(([r]) => r.action === 'addArtifact');
  expect(attempts).toHaveLength(2);
  expect(attempts[1][0]).toEqual(attempts[0][0]);
});

it('ignores a late artifact preview after a different version was selected', async () => {
  listing.artifacts.push({
    ...image,
    id: 'second',
    title: 'Second screenshot',
    sha256: 'b'.repeat(64),
  });
  let resolve!: (value: unknown) => void;
  const normal = request.getMockImplementation()!;
  request.mockImplementation((input) =>
    input.action === 'readArtifact' && input.artifactId === 'image'
      ? new Promise((r) => {
          resolve = r;
        })
      : normal(input),
  );
  render(view());
  await screen.findByLabelText('Saved artifact');
  fireEvent.change(screen.getByLabelText('Saved artifact'), { target: { value: 'image' } });
  await waitFor(() => expect(resolve).toBeTypeOf('function'));
  fireEvent.change(screen.getByLabelText('Saved artifact'), { target: { value: 'second' } });
  await screen.findByRole('img', { name: 'Second screenshot' });
  await act(async () =>
    resolve({ action: 'readArtifact', artifact: image, dataBase64: btoa('old') }),
  );
  expect(screen.getByRole('img', { name: 'Second screenshot' })).toBeInTheDocument();
  expect(screen.queryByRole('img', { name: 'Export screenshot' })).not.toBeInTheDocument();
});

it('requires reviewing an intervening alternative selection before overwriting it', async () => {
  listing.groups = [
    {
      id: 'group',
      workspaceId: 'work',
      intentRevision: 1,
      title: 'Approaches',
      purpose: 'Choose',
      budgetMinutes: 5,
      author: 'user',
      createdAt: '',
      alternatives: [
        { id: 'a', title: 'Alpha', hypothesis: 'A', artifactIds: [], executionIds: [] },
        { id: 'b', title: 'Beta', hypothesis: 'B', artifactIds: [], executionIds: [] },
      ],
    },
  ];
  render(view());
  fireEvent.click(await screen.findByText('Approaches · alternatives · intent 1'));
  fireEvent.change(screen.getByLabelText('Choose alternative'), { target: { value: 'a' } });
  fireEvent.change(screen.getByLabelText('Selection reason'), {
    target: { value: 'Local reasoning' },
  });
  listing.selections = [
    {
      id: 'remote-choice',
      workspaceId: 'work',
      intentRevision: 1,
      groupId: 'group',
      alternativeId: 'b',
      reason: 'Another client choice',
      author: 'user',
      createdAt: '',
    },
  ];
  fireEvent.click(screen.getByRole('button', { name: 'Refresh artifacts' }));
  await screen.findByText('Beta · selected');
  expect(screen.getByRole('button', { name: 'Record selection' })).toBeDisabled();
  expect(screen.getByLabelText('Selection reason')).toHaveValue('Local reasoning');
  fireEvent.click(screen.getByRole('button', { name: 'I reviewed the current selection' }));
  fireEvent.click(screen.getByRole('button', { name: 'Record selection' }));
  await waitFor(() =>
    expect(request).toHaveBeenCalledWith(
      expect.objectContaining({
        action: 'selectAlternative',
        expectedSelectionId: 'remote-choice',
        alternativeId: 'a',
      }),
    ),
  );
});

it('shows malformed text preview errors without crashing the surface', async () => {
  listing.artifacts = [{ ...image, kind: 'text', mimeType: 'text/plain' }];
  const normal = request.getMockImplementation()!;
  request.mockImplementation((input) =>
    input.action === 'readArtifact'
      ? Promise.resolve({
          action: 'readArtifact',
          artifact: listing.artifacts[0],
          dataBase64: '%%%',
        })
      : normal(input),
  );
  render(view());
  await screen.findByLabelText('Saved artifact');
  fireEvent.change(screen.getByLabelText('Saved artifact'), { target: { value: 'image' } });
  expect(await screen.findByRole('alert')).toBeInTheDocument();
  expect(screen.getByRole('heading', { name: 'Artifacts and exploration' })).toBeInTheDocument();
});
