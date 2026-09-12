#!/usr/bin/env python3
"""Root-owned, socket-gated control of this combined node's HTTPS proxy only."""
import http.server
import hmac
import json
import os
from pathlib import Path
import socketserver
import subprocess
import threading

SOCKET = Path('/run/workspacer-network/admin.sock')
STATE = Path('/data/combined/network-sharing.json')
TS = ['tailscale', '--socket=/run/tailscale-private/tailscaled.sock']
LOCK = threading.Lock()
AUTH = os.environ.get('WKS_NETWORK_ADMIN_TOKEN', '')


def run(args):
    return subprocess.run(TS + args, check=True, capture_output=True, text=True, timeout=15).stdout


def status():
    state = json.loads(run(['status', '--json']))
    serving = run(['serve', 'status', '--json'])
    return dict(available=state.get('BackendState') == 'Running',
                magicName=state.get('Self', {}).get('DNSName', '').rstrip('.') or None,
                serveActive='127.0.0.1:7895' in serving or 'localhost:7895' in serving,
                canServe=True)


def set_serve(enabled):
    if enabled:
        for port in ('443', '8443'):
            run(['serve', '--bg', '--https=' + port, 'http://127.0.0.1:7895'])
    else:
        # This instance belongs exclusively to the combined Workspacer node.
        run(['serve', 'reset'])
    temporary = STATE.with_suffix('.tmp')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump({'enabled': enabled}, stream)
    os.replace(temporary, STATE)
    return {'ok': True}


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass  # Never log request bodies or Tailscale's private status document.

    def reply(self, code, value):
        data = json.dumps(value).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def authorized(self):
        return bool(AUTH) and hmac.compare_digest(self.headers.get('Authorization', ''), 'Bearer ' + AUTH)

    def do_GET(self):
        if not self.authorized():
            return self.reply(401, {'error': 'Unauthorized'})
        if self.path != '/status':
            return self.reply(404, {'error': 'Unknown network operation'})
        try:
            with LOCK:
                value = status()
            self.reply(200, value)
        except Exception:
            self.reply(503, {'error': 'Tailscale status is unavailable'})

    def do_POST(self):
        if not self.authorized():
            return self.reply(401, {'error': 'Unauthorized'})
        if self.path != '/serve':
            return self.reply(404, {'error': 'Unknown network operation'})
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= 128:
                return self.reply(400, {'error': 'Invalid network request'})
            value = json.loads(self.rfile.read(length))
            if not isinstance(value, dict) or set(value) != {'enabled'} or type(value['enabled']) is not bool:
                return self.reply(400, {'error': 'Expected enabled boolean'})
            with LOCK:
                result = set_serve(value['enabled'])
            self.reply(200, result)
        except Exception:
            self.reply(503, {'error': 'Tailscale Serve change was not confirmed'})


class Server(socketserver.ThreadingMixIn, socketserver.UnixStreamServer):
    daemon_threads = True


if __name__ == '__main__':
    if not AUTH:
        raise RuntimeError('Private network credential is required')
    SOCKET.parent.mkdir(mode=0o711, parents=True, exist_ok=True)
    os.chmod(SOCKET.parent, 0o711)
    SOCKET.unlink(missing_ok=True)
    with Server(str(SOCKET), Handler) as server:
        server.serve_forever()
