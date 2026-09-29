#!/usr/bin/env python3
"""Probe an extracted release archive in isolated state, without Node on PATH.

No provider is invoked. A successful receipt proves packaged standalone startup,
authenticated service readiness and joined parent-pipe shutdown, not GUI parity.
"""
import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import socket
import subprocess
import tarfile
import tempfile
import time
from urllib.parse import urlsplit
import zipfile


class SmokeError(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise SmokeError(message)


def extract(archive, destination):
    """Release bundles contain ordinary files/directories; refuse links/traversal."""
    def target(name):
        path = PurePosixPath(name)
        require(not path.is_absolute() and '..' not in path.parts
                and '\\' not in name and ':' not in name,
                'unsafe archive member')
        require(path.parts and path.parts[0] == 'workspacer-server',
                'unexpected archive root')
        return destination.joinpath(*path.parts)

    if archive.suffix == '.zip':
        with zipfile.ZipFile(archive) as bundle:
            for member in bundle.infolist():
                path = target(member.filename)
                kind = (member.external_attr >> 16) & 0o170000
                require(kind in (0, 0o100000, 0o040000), 'archive links are unsupported')
                if member.is_dir():
                    path.mkdir(parents=True, exist_ok=True)
                else:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    with bundle.open(member) as source, path.open('wb') as out:
                        shutil.copyfileobj(source, out)
    else:
        with tarfile.open(archive, 'r:gz') as bundle:
            for member in bundle:
                path = target(member.name)
                require(member.isfile() or member.isdir(), 'archive links are unsupported')
                if member.isdir():
                    path.mkdir(parents=True, exist_ok=True)
                else:
                    path.parent.mkdir(parents=True, exist_ok=True)
                    with bundle.extractfile(member) as source, path.open('wb') as out:
                        shutil.copyfileobj(source, out)
                    path.chmod(member.mode & 0o777)
    return destination / 'workspacer-server'


def validate_bundle(bundle, commit, platform):
    suffix = '.exe' if platform == 'windows-x64' else ''
    for name in ('workspacer' + suffix, 'workspacer-rust' + suffix,
                 'claudemon' + suffix, 'web/index.html', 'README.md', 'build-stamp'):
        require((bundle / name).is_file(), 'missing server payload: ' + name)
    require((bundle / 'examples').is_dir(), 'missing plugin examples')
    fields = {}
    for line in (bundle / 'build-stamp').read_text().splitlines():
        key, separator, value = line.partition('=')
        require(separator and key not in fields, 'invalid build stamp')
        fields[key] = value
    for key, value in {'component': 'server', 'install': 'release',
                       'commit': commit, 'platform': platform}.items():
        require(fields.get(key) == value, 'build stamp mismatch: ' + key)
    alias = bundle / ('workspacer' + suffix)
    backend = bundle / ('workspacer-rust' + suffix)
    def digest(path):
        with path.open('rb') as stream:
            digest = hashlib.sha256()
            for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                digest.update(chunk)
            return digest.digest()
    require(digest(alias) == digest(backend), 'CLI alias differs from Rust backend')
    for legacy in ('hub', 'brain', 'mcp-facade', 'desktop-host.cjs', 'node'):
        require(not (bundle / legacy).exists() and not (bundle / (legacy + '.exe')).exists(),
                'retired runtime present: ' + legacy)
    return alias


def isolated_environment(root):
    # An allowlist keeps developer/CI credentials, provider homes, proxy settings,
    # ambient hub addresses and machine-power endpoints out of the fixture.
    env = {key: value for key, value in os.environ.items()
           if key.upper() in ('SYSTEMROOT', 'WINDIR', 'SYSTEMDRIVE', 'COMSPEC',
                              'PROCESSOR_ARCHITECTURE', 'NUMBER_OF_PROCESSORS')}
    for name, relative in {'HOME': 'home', 'USERPROFILE': 'home', 'APPDATA': 'appdata',
                           'LOCALAPPDATA': 'localappdata', 'XDG_CONFIG_HOME': 'xdg-config',
                           'XDG_DATA_HOME': 'xdg-data', 'XDG_CACHE_HOME': 'xdg-cache',
                           'TMPDIR': 'tmp', 'TMP': 'tmp', 'TEMP': 'tmp', 'PATH': 'empty-bin'}.items():
        directory = root / relative
        directory.mkdir(exist_ok=True)
        env[name] = str(directory)
    env.update(WORKSPACER_PARENT_PID=str(os.getpid()), WORKSPACER_USAGE_POLL_ON_BOOT='0')
    return env


def endpoint(value, scheme, path):
    parsed = urlsplit(value)
    require(parsed.scheme == scheme and parsed.hostname == '127.0.0.1'
            and parsed.port and parsed.path == path and not parsed.query
            and not parsed.fragment and not parsed.username and not parsed.password,
            'unexpected readiness endpoint')
    return parsed.port


def get(port, path, token=None):
    # http.client neither follows redirects nor consults proxy environment.
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
    try:
        connection.request('GET', path, headers={'Authorization': 'Bearer ' + token} if token else {})
        response = connection.getresponse()
        body = response.read(4 * 1024 * 1024 + 1)
        require(response.status == 200 and len(body) <= 4 * 1024 * 1024,
                'HTTP readiness failed: ' + path)
        return body, response.getheader('X-Workspacer-Maintenance')
    finally:
        connection.close()


def await_banner(process, stdout, budget):
    deadline = time.monotonic() + budget
    while time.monotonic() < deadline:
        require(process.poll() is None, f'server exited before readiness (exit {process.returncode})')
        raw = stdout.read_bytes()
        require(len(raw) < 65536, 'oversized readiness banner')
        if b'\n' in raw:
            try:
                return json.loads(raw.split(b'\n', 1)[0])
            except (ValueError, UnicodeError):
                raise SmokeError('invalid readiness JSON') from None
        time.sleep(0.05)
    raise SmokeError('server readiness timed out')


def mcp_catalog(port, token, version):
    params = {}
    if version == '2026-07-28':
        params['_meta'] = {
            'io.modelcontextprotocol/protocolVersion': version,
            'io.modelcontextprotocol/clientInfo': {'name': 'release-smoke', 'version': '1'},
            'io.modelcontextprotocol/clientCapabilities': {},
        }
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=5)
    try:
        connection.request('POST', '/mcp', body=json.dumps({
            'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list', 'params': params,
        }), headers={
            'Authorization': 'Bearer ' + token,
            'Content-Type': 'application/json',
            'Accept': 'application/json, text/event-stream',
            'MCP-Protocol-Version': version,
            'Mcp-Method': 'tools/list',
        })
        response = connection.getresponse()
        raw = response.read(4 * 1024 * 1024 + 1)
        require(response.status == 200 and len(raw) <= 4 * 1024 * 1024,
                'MCP tools/list transport failed: ' + version)
        body = json.loads(raw)
        require(isinstance(body, dict) and body.get('jsonrpc') == '2.0'
                and type(body.get('id')) is int and body['id'] == 1,
                'MCP tools/list response envelope mismatch: ' + version)
        require('error' not in body and isinstance(body.get('result'), dict),
                'MCP tools/list result failed: ' + version)
        result = body['result']
        if version == '2026-07-28':
            require(result.get('resultType') == 'complete', 'MCP tools/list resultType missing')
            require(type(result.get('ttlMs')) is int and result['ttlMs'] == 0
                    and result.get('cacheScope') == 'private',
                    'MCP tools/list missing private cache hints: ' + version)
        tools = result.get('tools')
        require(isinstance(tools, list) and any(tool.get('name') == 'get_host_cwd' for tool in tools),
                'MCP tools/list catalog missing: ' + version)
        return len(tools)
    finally:
        connection.close()


def released(ports):
    # Refuse success while ANY previously healthy owned listener accepts.
    for port in ports:
        with socket.socket() as probe:
            probe.settimeout(0.5)
            require(probe.connect_ex(('127.0.0.1', port)) != 0,
                    'owned listener remained after server exit')
        # Unix SO_REUSEADDR permits TIME_WAIT while still refusing an active
        # listener. Windows exclusive rebinding can reject closed TIME_WAIT
        # sockets, and reuse-address can bind over live listeners: successful
        # process join plus refused connection above is the Windows contract.
        if os.name != 'nt':
            with socket.socket() as probe:
                probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                probe.bind(('127.0.0.1', port))



def diagnostic_tail(path, token):
    # Only the isolated child's stderr is eligible. Never include stdout or the
    # status JSON, which can contain the pairing credential and complete URLs.
    with path.open('rb') as stream:
        stream.seek(max(0, path.stat().st_size - 2048))
        raw = stream.read().decode('utf-8', errors='replace').replace(token, '[redacted]')
    lines = []
    for line in raw.splitlines()[-8:]:
        if any(word in line.lower() for word in ('token', 'credential', 'secret', 'password', 'authorization')):
            line = '[credential-bearing stderr line omitted]'
        lines.append(''.join(char for char in line if char.isprintable()))
    return ' | '.join(lines) or '(no stderr)'


def smoke(command, bundle, root, timeout=45):
    env = isolated_environment(root)
    config, data = root / 'config', root / 'data'
    config.mkdir()
    data.mkdir()
    # Config-file precedence wins over the environment in the launcher.
    (config / 'config.yaml').write_text('usage:\n  pollOnBoot: false\n')
    token = os.urandom(32).hex()
    (config / 'remote-token').write_text(token)
    database = root / 'engine.sqlite'
    reservations = [socket.socket(), socket.socket()]
    try:
        for item in reservations:
            item.bind(('127.0.0.1', 0))
        api, hook = [item.getsockname()[1] for item in reservations]
    finally:
        for item in reservations:
            item.close()
    argv = command + ['serve', '--json', '--host', '127.0.0.1', '--hub-port', '0',
                      '--mcp-port', '0', '--claudemon-api-port', str(api),
                      '--claudemon-hook-port', str(hook), '--no-claudemon-init',
                      '--config-dir', str(config), '--home-dir', env['HOME'],
                      '--data-dir', str(data), '--claudemon-db-path', str(database)]
    stdout = root / 'stdout.log'
    with stdout.open('wb') as out, (root / 'stderr.log').open('wb') as err:
        process = subprocess.Popen(argv, cwd=bundle, env=env, stdin=subprocess.PIPE,
                                   stdout=out, stderr=err)
        try:
            banner = await_banner(process, stdout, timeout)
            require(banner.get('service') == 'workspacer-rust'
                    and banner.get('mode') == 'standalone', 'wrong server identity or mode')
            require(banner.get('token') == token, 'server did not retain isolated credential')
            require(Path(banner.get('database', '')).resolve() == database.resolve(),
                    'server selected another database')
            hub = endpoint(banner['hubUrl'], 'http', '')
            require(endpoint(banner['busUrl'], 'ws', '/bus') == hub, 'bus/hub mismatch')
            require(endpoint(banner['claudemonUrl'], 'http', '') == api, 'engine port mismatch')
            mcp = endpoint(banner['mcpUrl'], 'http', '/mcp')
            ports = [hub, mcp, api, hook]
            require(len(set(ports)) == 4, 'listeners are not distinct')
            health = json.loads(get(hub, '/health', token)[0])
            require(health.get('status') == 'ok' and isinstance(health.get('methods'), int)
                    and health['methods'] > 0, 'authenticated hub health is incomplete')
            require('methods' not in json.loads(get(hub, '/health')[0]),
                    'hub exposes authenticated health without credential')
            require(get(api, '/health') == (b'ok', '1'), 'engine identity mismatch')
            require(get(hook, '/health')[0] == b'ok', 'hook listener is not ready')
            facade = json.loads(get(mcp, '/health')[0])
            expected = {'status': 'ok', 'service': 'workspacer-mcp-facade',
                        'implementation': 'rust', 'hubConnected': True,
                        'pluginCatalogReady': True, 'listenAddr': f'127.0.0.1:{mcp}',
                        'hubUrl': banner['busUrl']}
            require(all(facade.get(k) == v for k, v in expected.items()),
                    'MCP identity, hub or initial catalog mismatch')
            for version in ('2026-07-28', '2025-11-25'):
                mcp_catalog(mcp, token, version)
            require(get(hub, '/app/', token)[0] == (bundle / 'web/index.html').read_bytes(),
                    'packaged sibling web entry was not served')
            # status issues an authenticated real WebSocket brain.info call.
            # Its exit code alone excludes brain failure, so check every field.
            status = subprocess.run(command + ['status', '--json', '--host', '127.0.0.1',
                                    '--hub-port', str(hub), '--claudemon-api-port', str(api),
                                    '--config-dir', str(config)], cwd=bundle, env=env,
                                    capture_output=True, timeout=15)
            require(status.returncode == 0, f'packaged status command failed (exit {status.returncode})')
            report = json.loads(status.stdout)
            require(all(report.get(key, {}).get('ok') is True
                        for key in ('hub', 'claudemon', 'brain')),
                    'authenticated service registration probe failed')
            require(process.poll() is None, 'server exited during probes')
            process.stdin.close()
            try:
                code = process.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                raise SmokeError('server did not join shutdown after parent-pipe EOF') from None
            require(code == 0, f'server shutdown returned failure (exit {code})')
            released(ports)
            require((config / 'remote-token').read_text() == token, 'credential changed')
            require(database.is_file(), 'isolated engine database missing')
            require(not (root / 'home/.claude/settings.json').exists(),
                    'disabled hook initialization wrote provider settings')
        except SmokeError as error:
            err.flush()
            raise SmokeError(str(error) + '; isolated stderr: '
                             + diagnostic_tail(root / 'stderr.log', token)) from None
        finally:
            if not process.stdin.closed:
                process.stdin.close()
            if process.poll() is None:
                # Cleanup never converts timeout/failure into a passing receipt.
                process.kill()
            process.wait(timeout=10)
    return {'service': 'workspacer-rust', 'mode': 'standalone', 'listeners': 4,
            'authenticatedBrainProbe': True, 'nodeOnPath': False, 'joinedShutdown': True,
            'mcpCatalogVersions': ['2026-07-28', '2025-11-25']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--release-dir', type=Path, required=True)
    parser.add_argument('--platform', choices=['windows-x64', 'darwin-arm64', 'linux-x64'], required=True)
    parser.add_argument('--commit', required=True)
    args = parser.parse_args()
    suffix = '.zip' if args.platform == 'windows-x64' else '.tar.gz'
    archive = args.release_dir / ('workspacer-server-' + args.platform + suffix)
    with tempfile.TemporaryDirectory(prefix='wks-packaged-smoke-') as temporary:
        root = Path(temporary)
        bundle = extract(archive, root / 'unpacked')
        executable = validate_bundle(bundle, args.commit, args.platform)
        state = root / 'isolated'
        state.mkdir()
        report = smoke([str(executable)], bundle, state)
        report.update(commit=args.commit, platform=args.platform, archive=archive.name)
        print(json.dumps(report, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (SmokeError, OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        # Banner/logs contain an isolated pairing token; do not dump them to CI.
        raise SystemExit('Packaged server smoke failed: ' + type(error).__name__
                         + (': ' + str(error) if isinstance(error, SmokeError) else '')) from None
