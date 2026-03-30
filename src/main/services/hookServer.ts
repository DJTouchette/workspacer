import http from 'http';
import { execSync } from 'child_process';
import { claudeSessionStore } from './claudeSessionStore';

const HOOK_PORT = 7890;

let server: http.Server | null = null;

/** Kill any process listening on HOOK_PORT (stale workspacer instance) */
function killStaleListener(): void {
  try {
    if (process.platform === 'win32') {
      const out = execSync(`netstat -ano | findstr "127.0.0.1:${HOOK_PORT}"`, { encoding: 'utf-8', timeout: 3000 });
      const match = out.match(/LISTENING\s+(\d+)/);
      if (match) {
        const pid = parseInt(match[1], 10);
        if (pid && pid !== process.pid) {
          console.log(`[HookServer] killing stale listener pid=${pid}`);
          execSync(`taskkill /F /PID ${pid}`, { timeout: 3000 });
        }
      }
    } else {
      const out = execSync(`lsof -ti :${HOOK_PORT}`, { encoding: 'utf-8', timeout: 3000 });
      for (const line of out.trim().split('\n')) {
        const pid = parseInt(line, 10);
        if (pid && pid !== process.pid) {
          console.log(`[HookServer] killing stale listener pid=${pid}`);
          process.kill(pid, 'SIGTERM');
        }
      }
    }
  } catch {
    // No listener found or kill failed — will retry bind anyway
  }
}

function createAndListen(): http.Server {
  server = http.createServer((req, res) => {
    if (req.method === 'POST' && req.url === '/hook') {
      let body = '';
      req.on('data', (chunk: string) => {
        body += chunk;
      });
      req.on('end', () => {
        try {
          const event = JSON.parse(body);
          const hookName = event.hook_event_name ?? event.type ?? 'unknown';
          console.log(`[HookServer] received: ${hookName} session=${event.session_id ?? '?'} cwd=${event.cwd ?? '?'}`);
          claudeSessionStore.handleHookEvent(event);
          res.writeHead(200);
          res.end('ok');
        } catch (err) {
          console.error('[HookServer] bad json:', err);
          res.writeHead(400);
          res.end('bad json');
        }
      });
    } else {
      res.writeHead(404);
      res.end();
    }
  });

  server.listen(HOOK_PORT, '127.0.0.1', () => {
    console.log(`[HookServer] listening on 127.0.0.1:${HOOK_PORT}`);
  });

  server.on('error', (err: NodeJS.ErrnoException) => {
    if (err.code === 'EADDRINUSE') {
      console.warn(`[HookServer] port ${HOOK_PORT} in use — killing stale listener and retrying...`);
      killStaleListener();
      setTimeout(() => {
        server!.listen(HOOK_PORT, '127.0.0.1', () => {
          console.log(`[HookServer] listening on 127.0.0.1:${HOOK_PORT} (reclaimed)`);
        });
      }, 1000);
    } else {
      console.error('[HookServer] error:', err);
    }
  });

  return server;
}

export function startHookServer(): http.Server {
  if (server) return server;
  return createAndListen();
}

export function stopHookServer(): void {
  if (server) {
    server.close();
    server = null;
  }
}
