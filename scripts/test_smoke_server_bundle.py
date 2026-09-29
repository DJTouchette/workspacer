"""Failure-path tests use a real isolated subprocess with four HTTP listeners."""
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location('bundle_smoke', Path(__file__).with_name('smoke-server-bundle.py'))
smoke = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(smoke)


def fixture(mode):
    import http.server
    import threading
    import time
    args = sys.argv[3:]
    def argument(name):
        return args[args.index(name) + 1]
    if args[0] == 'status':
        print(json.dumps({key: {'ok': key != 'brain' or mode != 'brain-missing'}
                          for key in ('hub', 'claudemon', 'brain')}))
        return
    config = Path(argument('--config-dir'))
    token = (config / 'remote-token').read_text()
    Path(argument('--claudemon-db-path')).write_bytes(b'fixture')
    assert os.environ['PATH'].endswith('empty-bin')
    assert 'HUB_TOKEN' not in os.environ and 'OPENAI_API_KEY' not in os.environ
    assert '--no-claudemon-init' in args
    api, hook = int(argument('--claudemon-api-port')), int(argument('--claudemon-hook-port'))
    servers = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            assert self.server.server_port == mcp and self.path == '/mcp'
            assert self.headers.get('Authorization') == 'Bearer ' + token
            assert self.headers.get('Mcp-Method') == 'tools/list'
            request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            modern = self.headers.get('MCP-Protocol-Version') == '2026-07-28'
            if modern:
                meta = request['params']['_meta']
                assert meta['io.modelcontextprotocol/protocolVersion'] == '2026-07-28'
                assert meta['io.modelcontextprotocol/clientCapabilities'] == {}
                assert meta['io.modelcontextprotocol/clientInfo']['name'] == 'release-smoke'
            result = {'tools': [{'name': 'get_host_cwd'}]}
            if modern:
                result['resultType'] = 'complete'
                if mode != 'mcp-cache-missing':
                    result.update(ttlMs=0, cacheScope='private')
                if mode == 'mcp-cache-public':
                    result['cacheScope'] = 'public'
            body = json.dumps({'jsonrpc': '2.0',
                               'id': 999 if mode == 'mcp-wrong-id' else request['id'],
                               'result': result}).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def do_GET(self):
            port = self.server.server_port
            authorized = self.headers.get('Authorization') == 'Bearer ' + token
            if port == api or port == hook:
                body = b'ok'
            elif port == hub:
                if self.path == '/app/':
                    body = Path('web/index.html').read_bytes()
                else:
                    body = json.dumps({'status': 'ok', **({'methods': 100} if authorized else {})}).encode()
            else:
                body = json.dumps({'status': 'ok', 'service': 'workspacer-mcp-facade',
                                   'implementation': 'rust', 'hubConnected': True,
                                   'pluginCatalogReady': mode != 'catalog-missing',
                                   'listenAddr': f'127.0.0.1:{mcp}',
                                   'hubUrl': f'ws://127.0.0.1:{hub}/bus'}).encode()
            self.send_response(200)
            if port == api:
                self.send_header('X-Workspacer-Maintenance', '1')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def log_message(self, *args):
            pass
    for port in (0, 0, api, hook):
        servers.append(http.server.ThreadingHTTPServer(('127.0.0.1', port), Handler))
    hub, mcp = [item.server_port for item in servers[:2]]
    workers = [threading.Thread(target=item.serve_forever, kwargs={'poll_interval': .05}) for item in servers]
    for worker in workers:
        worker.start()
    banner = {'service': 'workspacer-rust', 'mode': 'standalone', 'token': token,
              'database': argument('--claudemon-db-path'),
              'hubUrl': f'http://127.0.0.1:{hub}', 'busUrl': f'ws://127.0.0.1:{hub}/bus',
              'claudemonUrl': f'http://127.0.0.1:{api}', 'mcpUrl': f'http://127.0.0.1:{mcp}/mcp'}
    if mode == 'wrong-endpoint':
        banner['hubUrl'] = 'http://example.invalid:1'
    print(json.dumps(banner), flush=True)
    sys.stdin.read()
    if mode == 'hang':
        time.sleep(60)
    for item in servers:
        item.shutdown()
        item.server_close()
    for worker in workers:
        worker.join()
    if mode == 'bad-exit':
        raise SystemExit(7)


class BundleSmokeTests(unittest.TestCase):
    def test_full_lifecycle_and_failures_use_real_processes(self):
        cases = [('ok', None), ('brain-missing', 'registration'),
                 ('catalog-missing', 'catalog'), ('wrong-endpoint', 'endpoint'),
                 ('mcp-cache-missing', 'cache hints'), ('mcp-cache-public', 'cache hints'),
                 ('mcp-wrong-id', 'envelope mismatch'),
                 ('bad-exit', 'shutdown returned'), ('hang', 'join shutdown')]
        for mode, message in cases:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                bundle, state = root / 'bundle', root / 'state'
                (bundle / 'web').mkdir(parents=True)
                (bundle / 'web/index.html').write_text('<html>fixture</html>')
                state.mkdir()
                command = [sys.executable, '-B', str(Path(__file__).resolve()), '--fixture', mode]
                with patch.dict(os.environ, {'HUB_TOKEN': 'ambient', 'OPENAI_API_KEY': 'ambient'}):
                    if message:
                        with self.assertRaisesRegex(smoke.SmokeError, message):
                            smoke.smoke(command, bundle, state, timeout=2)
                    else:
                        report = smoke.smoke(command, bundle, state, timeout=2)
                        self.assertTrue(report['joinedShutdown'])
                        self.assertEqual(report['listeners'], 4)

    def test_diagnostics_redact_credentials_and_preserve_startup_reason(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'stderr'
            path.write_text('bind failed: address unavailable\npairing token=opaque\nunlabelled opaque\n')
            result = smoke.diagnostic_tail(path, 'opaque')
            self.assertNotIn('opaque', result)
            self.assertIn('bind failed: address unavailable', result)
            self.assertIn('[redacted]', result)

    def test_live_listener_cannot_be_reported_released(self):
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            listener.listen()
            with self.assertRaisesRegex(smoke.SmokeError, 'remained'):
                smoke.released([listener.getsockname()[1]])

    def test_archives_extract_payload_and_preserve_executable_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / 'bundle.tar.gz'
            with tarfile.open(archive, 'w:gz') as bundle:
                member = tarfile.TarInfo('workspacer-server/workspacer')
                member.mode, member.size = 0o755, 3
                bundle.addfile(member, io.BytesIO(b'bin'))
            extracted = smoke.extract(archive, root / 'tar')
            self.assertEqual((extracted / 'workspacer').read_bytes(), b'bin')
            if os.name != 'nt':
                self.assertEqual((extracted / 'workspacer').stat().st_mode & 0o777, 0o755)
            archive = root / 'bundle.zip'
            with zipfile.ZipFile(archive, 'w') as bundle:
                bundle.writestr('workspacer-server/workspacer.exe', b'bin')
            extracted = smoke.extract(archive, root / 'zip')
            self.assertEqual((extracted / 'workspacer.exe').read_bytes(), b'bin')

    def test_archives_refuse_traversal_and_links(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for name in ('../escape', '/absolute', 'workspacer-server/../../escape', 'C:/escape',
                         'workspacer-server/..\\escape'):
                with self.subTest(name=name):
                    archive = root / 'bad.zip'
                    with zipfile.ZipFile(archive, 'w') as bundle:
                        bundle.writestr(name, b'bad')
                    with self.assertRaises(smoke.SmokeError):
                        smoke.extract(archive, root / 'out')
            archive = root / 'link.tar.gz'
            with tarfile.open(archive, 'w:gz') as bundle:
                member = tarfile.TarInfo('workspacer-server/link')
                member.type, member.linkname = tarfile.SYMTYPE, '/outside'
                bundle.addfile(member)
            with self.assertRaisesRegex(smoke.SmokeError, 'links'):
                smoke.extract(archive, root / 'out')

    def test_stamp_payload_alias_and_retired_runtime_checks(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'web').mkdir()
            (root / 'examples').mkdir()
            for name in ('workspacer', 'workspacer-rust', 'claudemon', 'web/index.html', 'README.md'):
                (root / name).write_bytes(b'fixture')
            stamp = 'component=server\ninstall=release\ncommit=abc\nplatform=linux-x64\n'
            (root / 'build-stamp').write_text(stamp)
            self.assertEqual(smoke.validate_bundle(root, 'abc', 'linux-x64'), root / 'workspacer')
            with self.assertRaisesRegex(smoke.SmokeError, 'commit'):
                smoke.validate_bundle(root, 'different', 'linux-x64')
            (root / 'workspacer').write_bytes(b'wrong')
            with self.assertRaisesRegex(smoke.SmokeError, 'alias'):
                smoke.validate_bundle(root, 'abc', 'linux-x64')
            (root / 'workspacer').write_bytes(b'fixture')
            (root / 'node.exe').write_bytes(b'legacy')
            with self.assertRaisesRegex(smoke.SmokeError, 'retired'):
                smoke.validate_bundle(root, 'abc', 'linux-x64')
            (root / 'node.exe').unlink()
            (root / 'web/index.html').unlink()
            with self.assertRaisesRegex(smoke.SmokeError, 'missing'):
                smoke.validate_bundle(root, 'abc', 'linux-x64')


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--fixture':
        # command prefix is python THIS --fixture MODE; pass service args only.
        mode = sys.argv[2]
        fixture(mode)
    else:
        unittest.main()
