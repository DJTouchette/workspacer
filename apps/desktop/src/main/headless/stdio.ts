import { acceptHostResult } from './hostBridge';
import { createInterface } from 'node:readline';
import { desktopHostCall, setDesktopEventSink } from './desktopHost';

// stdout belongs exclusively to the private protocol. The bundle banner also
// redirects module-initialization logs before imports run.
console.log = (...args) => console.error(...args);
setDesktopEventSink((event, data) => process.stdout.write(JSON.stringify({ event, data }) + '\n'));
const input = createInterface({ input: process.stdin, crlfDelay: Infinity });
let active = 0;
let closed = false;
input.on('line', (line) => {
  let request: { id: string; method: string; params: Record<string, unknown>; context: Parameters<typeof desktopHostCall>[2] };
  try { request = JSON.parse(line); } catch { process.exitCode = 1; input.close(); return; }
  if (acceptHostResult(request)) return;
  if (typeof request.id !== 'string' || typeof request.method !== 'string' || !request.context) { process.exitCode = 1; input.close(); return; }
  active++;
  void desktopHostCall(request.method, request.params ?? {}, request.context).then(
    (result) => process.stdout.write(JSON.stringify({ id: request.id, result: result ?? null }) + '\n'),
    (error) => process.stdout.write(JSON.stringify({ id: request.id, error: error instanceof Error ? error.message : 'Desktop service failed' }) + '\n'),
  ).finally(() => { active--; if (closed && active === 0) process.exit(); });
});
input.on('close', () => { closed = true; if (active === 0) process.exit(); });
