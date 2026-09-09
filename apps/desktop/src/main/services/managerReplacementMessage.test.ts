import { expect, it, vi, afterEach } from 'vitest';
import { randomUUID } from 'crypto';
vi.mock('electron', () => ({ BrowserWindow: class {}, MessageChannelMain: class {} }));
vi.mock('./configService', () => ({ getConfigDir: () => process.env.TMPDIR! }));
vi.mock('./claudemonDaemon', () => ({ CLAUDEMON_API_URL: 'http://127.0.0.1:0' }));
vi.mock('./systemNotice', () => ({ notifySystem: vi.fn() }));
import { claudemonSessionClient } from './claudemonSessionClient';
import { managerReplacementState } from './managerReplacementState';
afterEach(() => vi.unstubAllGlobals());

it('correlates identical in-flight messages independently and persists missing acknowledgement without retry', async () => {
  const source = randomUUID();
  let first!: (value: Response) => void;
  let second!: (error: Error) => void;
  const fetch = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<Response>((resolve) => {
          first = resolve;
        }),
    )
    .mockImplementationOnce(
      () =>
        new Promise<Response>((_, reject) => {
          second = reject;
        }),
    );
  vi.stubGlobal('fetch', fetch);
  const accepted = claudemonSessionClient.message(source, 'identical message');
  const uncertain = claudemonSessionClient.message(source, 'identical message');
  const rejected = expect(uncertain).rejects.toThrow('lost acknowledgement');
  const frames = claudemonSessionClient.inFlightMessages(source);
  expect(frames).toHaveLength(2);
  expect(frames[0].id).not.toBe(frames[1].id);
  const operationId = randomUUID();
  managerReplacementState.edit((d) =>
    d.operations.push({
      operationId,
      sourceSessionId: source,
      successorSessionId: randomUUID(),
      workspaceId: 'fixture',
      paneId: 'fixture',
      phase: 'preparing',
      createdAt: 1,
      updatedAt: 1,
      committed: false,
      bound: false,
      artifactPath: '/fixture/proposal.json',
      workerIds: [],
      taskIds: [],
      metadata: [],
      signatures: {},
      finishes: {},
      launch: {
        options: {
          provider: 'codex',
          manager: true,
          toolScope: 'operator',
          cwd: process.env.TMPDIR!,
        },
        grants: 'fixture',
      },
      deliveries: frames.map((frame) => ({ ...frame, kind: 'message', status: 'sending' })),
    }),
  );
  first(new Response('{}', { status: 200 }));
  second(new Error('lost acknowledgement'));
  await accepted;
  await rejected;
  await claudemonSessionClient.waitForMessages([source]);
  const op = managerReplacementState.get(operationId);
  expect(op.deliveries.map((d) => [d.id, d.status])).toEqual([
    [frames[0].id, 'accepted'],
    [frames[1].id, 'uncertain'],
  ]);
  expect(op.phase).toBe('recovery-required');
  expect(fetch).toHaveBeenCalledTimes(2);
  expect(claudemonSessionClient.inFlightMessages(source)).toEqual([]);
});
