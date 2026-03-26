import http from 'http';
import { claudeSessionStore } from './claudeSessionStore';

const HOOK_PORT = 7890;

let server: http.Server | null = null;

export function startHookServer(): http.Server {
  if (server) return server;

  server = http.createServer((req, res) => {
    if (req.method === 'POST' && req.url === '/hook') {
      let body = '';
      req.on('data', (chunk: string) => {
        body += chunk;
      });
      req.on('end', () => {
        try {
          const event = JSON.parse(body);
          claudeSessionStore.handleHookEvent(event);
          res.writeHead(200);
          res.end('ok');
        } catch {
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

  // Don't crash if port is busy
  server.on('error', (err: NodeJS.ErrnoException) => {
    if (err.code === 'EADDRINUSE') {
      console.warn(`[HookServer] port ${HOOK_PORT} already in use — hooks will route to existing listener`);
    } else {
      console.error('[HookServer] error:', err);
    }
  });

  return server;
}

export function stopHookServer(): void {
  if (server) {
    server.close();
    server = null;
  }
}
