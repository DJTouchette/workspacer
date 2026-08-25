import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createWebBackend } from '../../src/backend/webBackend';

/**
 * Composer attachments on the web/remote backend.
 *
 * The desktop attaches by HOST PATH — picker, drop and paste all end in a path
 * the agent opens itself. A browser has no host path: the file is on the
 * viewer's machine. The old web `pickFiles` papered over that with a
 * `window.prompt` asking the user to TYPE paths on the host, which looks like a
 * feature and attaches nothing that exists. The hub already has the real
 * capability — `files.upload` (cmd/hub/upload.go), which /m has used for photo
 * attachments since it shipped — so the browser sends bytes and gets back a
 * path on the machine that runs the agent.
 */

const busMock = vi.hoisted(() => {
  class FakeHubBusClient {
    calls: { method: string; params: any; timeoutMs?: number }[] = [];
    fail: string | null = null;

    constructor(
      readonly token: string,
      readonly busUrl?: string,
    ) {}
    start() {}
    isConnected() {
      return true;
    }
    onStatus() {
      return () => {};
    }
    onReconnect() {
      return () => {};
    }
    subscribe() {
      return () => {};
    }
    call(method: string, params: any = {}, timeoutMs?: number) {
      this.calls.push({ method, params, timeoutMs });
      if (method === 'files.upload' || method.endsWith('/files.upload')) {
        if (this.fail) return Promise.reject(new Error(this.fail));
        return Promise.resolve({
          path: `/tmp/workspacer-uploads/m-1-abcd-${params.name}`,
          size: 3,
        });
      }
      return Promise.resolve({});
    }
  }
  const instances: FakeHubBusClient[] = [];
  return {
    instances,
    Ctor: class extends FakeHubBusClient {
      constructor(token: string, busUrl?: string) {
        super(token, busUrl);
        instances.push(this);
      }
    },
  };
});

vi.mock('../../src/backend/hubBusClient', () => ({ HubBusClient: busMock.Ctor }));

function bus() {
  return busMock.instances[busMock.instances.length - 1];
}

/** Drive the hidden <input type=file> the browser picker creates. */
function choose(files: File[]): void {
  const input = document.querySelector('input[type=file]') as HTMLInputElement | null;
  if (!input) throw new Error('no file input was opened');
  Object.defineProperty(input, 'files', { value: files, configurable: true });
  input.dispatchEvent(new Event('change'));
}

beforeEach(() => {
  busMock.instances.length = 0;
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('webBackend.pickFiles — a real browser picker, not a prompt for host paths', () => {
  it('refuses a HOST-path pick out loud rather than accepting a typed lie', async () => {
    const prompt = vi.fn().mockReturnValue('/usr/local/bin/claude');
    vi.stubGlobal('prompt', prompt);
    const posted: any[] = [];
    window.addEventListener('wks:notify-post', (e) => posted.push((e as CustomEvent).detail));
    const api = createWebBackend('t');
    // No `attachment` — the editor fallback and the custom-binary browsers want
    // a path on the machine running the agent, which a browser cannot produce.
    await expect(api.pickFiles()).resolves.toEqual([]);
    expect(prompt).not.toHaveBeenCalled();
    expect(posted.length, 'a refusal the caller cannot see is the old bug').toBe(1);
    expect(String(posted[0].title)).toMatch(/desktop app/i);
  });

  it('never asks the user to type a path on the host', async () => {
    const prompt = vi.fn().mockReturnValue('/etc/passwd');
    vi.stubGlobal('prompt', prompt);
    const api = createWebBackend('t');
    const pending = api.pickFiles(undefined, { attachment: true });
    expect(
      prompt,
      'the web picker must never ask for a path on someone else\u2019s host',
    ).not.toHaveBeenCalled();
    choose([new File(['abc'], 'shot.png', { type: 'image/png' })]);
    await pending;
  });

  it('uploads the chosen bytes and returns the path the hub wrote them to', async () => {
    const api = createWebBackend('t');
    const pending = api.pickFiles(undefined, { attachment: true });
    choose([new File(['abc'], 'shot.png', { type: 'image/png' })]);
    const paths = await pending;

    const upload = bus().calls.find((c) => c.method === 'files.upload');
    expect(upload, 'pickFiles must go through the hub files.upload capability').toBeTruthy();
    expect(upload!.params.name).toBe('shot.png');
    expect(upload!.params.dataBase64).toBe(btoa('abc'));
    expect(paths).toEqual(['/tmp/workspacer-uploads/m-1-abcd-shot.png']);
  });

  it('resolves empty when the picker is dismissed', async () => {
    const api = createWebBackend('t');
    const pending = api.pickFiles(undefined, { attachment: true });
    const input = document.querySelector('input[type=file]') as HTMLInputElement;
    input.dispatchEvent(new Event('cancel'));
    await expect(pending).resolves.toEqual([]);
  });

  it('refuses a file the hub would reject, loudly, without spending the round trip', async () => {
    const posted: any[] = [];
    window.addEventListener('wks:notify-post', (e) => posted.push((e as CustomEvent).detail));
    const api = createWebBackend('t');
    const pending = api.pickFiles(undefined, { attachment: true });
    choose([new File(['abc'], 'notes.txt', { type: 'text/plain' })]);
    const paths = await pending;

    expect(paths).toEqual([]);
    expect(bus().calls.some((c) => c.method === 'files.upload')).toBe(false);
    expect(posted.length, 'a refused attachment must be visible, not silent').toBe(1);
    expect(posted[0].level).toBe('error');
    expect(String(posted[0].body)).toMatch(/notes\.txt/);
  });

  it('surfaces an upload the hub refused instead of silently attaching nothing', async () => {
    const posted: any[] = [];
    window.addEventListener('wks:notify-post', (e) => posted.push((e as CustomEvent).detail));
    const api = createWebBackend('t');
    const pending = api.pickFiles(undefined, { attachment: true });
    bus().fail = 'files.upload: payload exceeds 24 MiB';
    choose([new File(['abc'], 'huge.png', { type: 'image/png' })]);

    await expect(pending).resolves.toEqual([]);
    expect(posted.length).toBe(1);
    expect(String(posted[0].body)).toMatch(/exceeds 24 MiB/);
  });
});

describe('webBackend.uploadAttachment — the primitive drop and paste share', () => {
  it('calls the hub-local files.upload with a generous timeout (bytes cross a link)', async () => {
    const api = createWebBackend('t');
    await api.uploadAttachment!({ name: 'a.png', dataBase64: 'AAA' });
    const upload = bus().calls.find((c) => c.method === 'files.upload')!;
    expect(upload.params).toEqual({ name: 'a.png', dataBase64: 'AAA' });
    expect(upload.timeoutMs ?? 0).toBeGreaterThan(15_000);
  });

  it('lands the bytes on the peer that runs the agent for a federated session', async () => {
    const api = createWebBackend('t');
    // Learn the session's hub the way every other qualified call does.
    await api.uploadAttachment!({ name: 'a.png', dataBase64: 'AAA', sessionId: 'unknown-sess' });
    expect(bus().calls.some((c) => c.method === 'files.upload')).toBe(true);
  });
});

describe('webBackend.openExternalUrl — a notification URL is not a dead click', () => {
  it('opens the link in the browser the user is actually sitting at', async () => {
    const open = vi.fn();
    vi.stubGlobal('open', open);
    const api = createWebBackend('t');
    expect(typeof api.openExternalUrl).toBe('function');
    await api.openExternalUrl!('https://example.com/x');
    expect(open).toHaveBeenCalledWith('https://example.com/x', '_blank', 'noopener,noreferrer');
  });
});

describe('webBackend.listRecentAgentSessions — an unavailable history is not an empty one', () => {
  it('propagates the failure instead of answering [] as if there were no history', async () => {
    const api = createWebBackend('t');
    const client = bus() as any;
    client.call = (method: string) =>
      method === 'sessions.recent'
        ? Promise.reject(new Error('no provider for sessions.recent'))
        : Promise.resolve({});
    await expect(api.listRecentAgentSessions()).rejects.toThrow(/no provider/);
  });
});
