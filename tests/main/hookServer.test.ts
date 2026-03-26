import { describe, it, expect, afterAll, vi, beforeAll } from 'vitest';
import http from 'http';

// Mock electron
vi.mock('electron', () => ({
  BrowserWindow: vi.fn(),
}));

// Mock the session store to capture events
const handleHookEventSpy = vi.fn();
vi.mock('../../src/main/services/claudeSessionStore', () => ({
  claudeSessionStore: {
    handleHookEvent: handleHookEventSpy,
    setMainWindow: vi.fn(),
  },
}));

const { startHookServer, stopHookServer } = await import('../../src/main/services/hookServer');

function postToHook(data: any): Promise<{ statusCode: number; body: string }> {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify(data);
    const req = http.request(
      {
        hostname: '127.0.0.1',
        port: 7890,
        path: '/hook',
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(body),
        },
      },
      (res) => {
        let responseBody = '';
        res.on('data', (chunk) => { responseBody += chunk; });
        res.on('end', () => {
          resolve({ statusCode: res.statusCode ?? 0, body: responseBody });
        });
      },
    );
    req.on('error', reject);
    req.write(body);
    req.end();
  });
}

describe('HookServer', () => {
  let server: http.Server;

  beforeAll(async () => {
    server = startHookServer();
    // Wait for server to be listening
    await new Promise<void>((resolve) => {
      if (server.listening) {
        resolve();
      } else {
        server.on('listening', resolve);
      }
    });
  });

  afterAll(() => {
    stopHookServer();
  });

  it('should accept POST /hook with valid JSON and forward to session store', async () => {
    const event = {
      hook_event_name: 'SessionStart',
      session_id: 'test-session-1',
      cwd: '/test/project',
    };

    const res = await postToHook(event);
    expect(res.statusCode).toBe(200);
    expect(res.body).toBe('ok');
    expect(handleHookEventSpy).toHaveBeenCalledWith(event);
  });

  it('should return 400 for invalid JSON', async () => {
    const res = await new Promise<{ statusCode: number; body: string }>((resolve, reject) => {
      const req = http.request(
        {
          hostname: '127.0.0.1',
          port: 7890,
          path: '/hook',
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
        },
        (res2) => {
          let body = '';
          res2.on('data', (chunk) => { body += chunk; });
          res2.on('end', () => resolve({ statusCode: res2.statusCode ?? 0, body }));
        },
      );
      req.on('error', reject);
      req.write('not valid json {{{');
      req.end();
    });

    expect(res.statusCode).toBe(400);
    expect(res.body).toBe('bad json');
  });

  it('should return 404 for non-hook paths', async () => {
    const res = await new Promise<{ statusCode: number }>((resolve, reject) => {
      const req = http.request(
        {
          hostname: '127.0.0.1',
          port: 7890,
          path: '/other',
          method: 'GET',
        },
        (res2) => {
          res2.resume();
          res2.on('end', () => resolve({ statusCode: res2.statusCode ?? 0 }));
        },
      );
      req.on('error', reject);
      req.end();
    });

    expect(res.statusCode).toBe(404);
  });

  it('should handle multiple concurrent hook events', async () => {
    handleHookEventSpy.mockClear();

    const events = Array.from({ length: 10 }, (_, i) => ({
      hook_event_name: 'PreToolUse',
      session_id: `concurrent-session-${i}`,
      cwd: `/test/concurrent/${i}`,
      tool_name: 'Bash',
      tool_use_id: `tu-concurrent-${i}`,
    }));

    const results = await Promise.all(events.map(e => postToHook(e)));

    expect(results.every(r => r.statusCode === 200)).toBe(true);
    expect(handleHookEventSpy).toHaveBeenCalledTimes(10);
  });
});
