import { afterEach, expect, it, vi } from 'vitest';
import fs from 'fs';
import os from 'os';
import path from 'path';
import { createServer } from 'http';
import { once } from 'events';
const transport = vi.hoisted(() => ({ url: '', service: undefined as any }));
vi.mock('electron', () => ({ BrowserWindow: class {}, MessageChannelMain: class {} }));
vi.mock('./claudemonDaemon', () => ({ get CLAUDEMON_API_URL() { return transport.url; } }));
vi.mock('./systemNotice', () => ({ notifySystem: vi.fn() }));
vi.mock('./managerReplacementState', () => ({ managerReplacementState: {
  holdMessage: vi.fn(() => false), noteInFlightMessage: vi.fn(), acknowledged: () => false,
} }));
vi.mock('./managerRequestService', async (original) => ({
  ...(await original<typeof import('./managerRequestService')>()), managerRequests: () => transport.service,
}));
import { ManagerRequestService } from './managerRequestService';
import { DispatchHistoryStore } from './dispatchHistoryStore';
import { claudemonSessionClient } from './claudemonSessionClient';
import { managerReplacementState } from './managerReplacementState';
const dirs: string[] = [];
afterEach(() => { vi.clearAllMocks(); dirs.splice(0).forEach((d) => fs.rmSync(d, { recursive: true, force: true })); });

it('uses real HTTP admission with unchanged text; delayed/lost/rejected acknowledgements retain logical identity', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'request-transport-'));
  dirs.push(dir);
  const history = new DispatchHistoryStore(() => path.join(dir, 'history.json'));
  transport.service = new ManagerRequestService(history,
    (id) => ({ sessionId: id, cwd: dir, status: 'active', isWakeTarget: true }),
    () => { throw new Error('No policy or provider in transport fixture'); });
  const bodies: unknown[] = [];
  let status = 200;
  let release: (() => void) | undefined;
  const server = createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(Buffer.from(chunk));
    bodies.push(JSON.parse(Buffer.concat(chunks).toString()));
    release = () => { res.writeHead(status, { 'content-type': 'application/json' }); res.end(JSON.stringify({ ok: status === 200, queued: true })); };
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  transport.url = `http://127.0.0.1:${(server.address() as any).port}`;
  const prepare = () => {
    const r = transport.service.prepare('manager', 'original user text');
    const d = transport.service.beginDelivery('manager', r.requestId);
    return { ...d, requestId: r.requestId };
  };
  const waitForRequest = async () => { while (!release) await new Promise((resolve) => setTimeout(resolve, 5)); };
  try {
    const a = prepare();
    const pending = claudemonSessionClient.message('manager', a.text, undefined, a);
    await waitForRequest();
    expect(history.listRequests('manager')[0].delivery).toBe('unknown');
    expect(transport.service.beginDelivery('manager', a.requestId)).toBeUndefined();
    release!(); release = undefined;
    expect(await pending).toMatchObject({ ok: true });
    expect(history.listRequests('manager')[0].delivery).toBe('accepted');
    await claudemonSessionClient.messageDirect('manager', a.text, a);
    expect(bodies).toEqual([{ text: 'original user text' }]);

    status = 409;
    const b = prepare();
    const rejected = claudemonSessionClient.message('manager', b.text, undefined, b);
    await waitForRequest(); release!(); release = undefined;
    expect(await rejected).toMatchObject({ ok: false });
    expect(transport.service.request('manager', b.requestId).delivery).toBe('rejected');
    const retry = transport.service.beginDelivery('manager', b.requestId);
    expect(retry.deliveryId).not.toBe(b.deliveryId);
    status = 500;
    const lost = claudemonSessionClient.message('manager', retry.text, undefined, { ...retry, requestId: b.requestId });
    const error = expect(lost).rejects.toThrow('500');
    await waitForRequest(); release!(); release = undefined;
    await error;
    expect(transport.service.request('manager', b.requestId).delivery).toBe('unknown');
    await expect(claudemonSessionClient.messageDirect('manager', retry.text, { ...retry, requestId: b.requestId })).rejects.toThrow(/must not replay/);
    expect(bodies).toHaveLength(3);

    vi.mocked(managerReplacementState.holdMessage).mockReturnValueOnce(true);
    const held = prepare();
    expect(await claudemonSessionClient.message('manager', held.text, undefined, held)).toMatchObject({ mode: 'handoff-queued' });
    expect(transport.service.request('manager', held.requestId).delivery).toBe('pending');
    history.adoptWorkflowTasks('manager', 'successor');
    status = 200;
    const transferred = claudemonSessionClient.messageDirect('successor', held.text, held);
    await waitForRequest(); release!(); release = undefined;
    await transferred;
    expect(transport.service.request('successor', held.requestId)).toMatchObject({ requestId: held.requestId, sourceSessionId: 'manager', delivery: 'accepted' });
    expect(bodies).toHaveLength(4);
  } finally {
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
