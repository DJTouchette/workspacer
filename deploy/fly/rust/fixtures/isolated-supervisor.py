#!/usr/bin/python3
"""Personal root supervisor. No stock bootstrap, secret inheritance or empty init."""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import secrets
import stat
import subprocess
import sys
import time
import urllib.request
from common import DATA, mount_guard, private_proc, private, real, require

ROOT = DATA / 'combined'
CHILDREN = []
PATH = '/usr/local/go/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin'
TS = ['tailscale', '--socket=/run/tailscale-private/tailscaled.sock']

def read_json(p):
    real(p)
    return json.loads(p.read_bytes())

def env_for(home):
    return dict(PATH=PATH, HOME=home, LANG='C.UTF-8',
        XDG_CONFIG_HOME=home+'/.config', XDG_DATA_HOME=home+'/.local/share',
        XDG_STATE_HOME=home+'/.local/state', XDG_CACHE_HOME=home+'/.cache')

def spawn(argv, env, uid, cwd, log):
    # setpriv clears all three capability sets and supplemental groups; NNP is inherited.
    prefix = ['/usr/bin/setpriv', '--reuid='+str(uid), '--regid='+str(uid),
        '--clear-groups', '--bounding-set=-all', '--inh-caps=-all', '--ambient-caps=-all', '--no-new-privs', '--']
    with open(log, 'ab', buffering=0) as output:
        p = subprocess.Popen(prefix+argv, env=env, cwd=cwd, start_new_session=True,
            stdin=subprocess.DEVNULL, stdout=output, stderr=output)
    CHILDREN.append(p)
    return p

def ready(url, proc):
    for _ in range(100):
        require(proc.poll() is None, 'service exited')
        try:
            with urllib.request.urlopen(url, timeout=1) as response:
                if response.status == 200:
                    return
        except Exception:
            pass
        time.sleep(0.2)
    raise RuntimeError('readiness timeout')

def stop(*_):
    for p in reversed(CHILDREN):
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGTERM)
    end = time.monotonic()+45
    while time.monotonic() < end and any(p.poll() is None for p in CHILDREN):
        time.sleep(0.2)
    for p in CHILDREN:
        if p.poll() is None:
            os.killpg(p.pid, signal.SIGKILL)
        p.wait()

def boot():
    mount_guard()
    private_proc()
    os.umask(0o077)
    # Subreaper also works under Fly init. Reap orphaned agent descendants below.
    require(ctypes.CDLL(None).prctl(36, 1, 0, 0, 0) == 0, 'subreaper unavailable')
    private(DATA, 0, 0, 0o711)
    private(ROOT)
    private(DATA/'hub', 0, 10002, 0o730)
    private(DATA/'tailscale')
    for name in ('home', 'go', 'cache', 'bundle', 'bun', 'npm', 'repos'):
        s = real(DATA/name, True)
        require(s.st_uid == 10001 and s.st_gid == 10001 and not s.st_mode & 0o022, 'worker directory audit failed')
    db = DATA/'home/.local/share/claudemon/state.db'
    real(db)
    require(db.stat().st_size > 0, 'missing worker state DB')
    active = read_json(ROOT/'activated.json')
    approval = read_json(ROOT/'runtime.json')
    require(approval['approved'] is True and approval['policy'] in ('always-on', 'manual-sleep'), 'cutover approval missing')
    require(approval['manifestSha256'] == active['manifestSha256'], 'approval manifest mismatch')
    require(approval['sourceStoppedAttestedAt'], 'source stop attestation missing')
    raw = (ROOT/'manifest.json').read_bytes()
    require(hashlib.sha256(raw).hexdigest() == active['manifestSha256'], 'activation evidence mismatch')
    # State can legitimately evolve after first boot: enforce presence/type on each restart.
    for e in json.loads(raw)['entries']:
        s = real(DATA/e['target'], e['type']=='dir')
        require(s.st_uid == e['targetUid'] and s.st_gid == e['targetGid'] and not s.st_mode & 0o077, 'migrated state permission drift')
        if e['source'] in ('tailscale/tailscaled.state', 'home/.config/workspacer/remote-token'):
            require(s.st_size > 0, 'credential/identity loss')
    cfg = DATA/'hub/home/.config/workspacer'
    require(not (cfg/'nodes.json').exists() and not (cfg/'nodes.json').is_symlink(), 'nodes registry forbidden')
    records = read_json(cfg/'tokens.json')
    host = (cfg/'remote-token').read_text().strip()
    require(bool(host), 'empty pairing token')
    def credential(label, scope):
        require(isinstance(label, str) and label, 'credential label missing')
        found = [r for r in records if r.get('label') == label]
        require(len(found) == 1 and found[0]['scope'] == scope, 'credential label/scope mismatch')
        require(found[0]['token'] and found[0]['token'] != host, 'host credential cannot be a worker credential')
        return found[0]
    provider = credential(approval['providerTokenLabel'], 'provider')
    require(approval['providerProvides'] and sorted(provider.get('provides', [])) == sorted(approval['providerProvides']), 'provider grant mismatch')
    require(approval['mcpScope'] in ('view', 'triage', 'operator'), 'MCP scope invalid')
    mcp = credential(approval['mcpTokenLabel'], approval['mcpScope'])
    require(mcp['token'] != provider['token'], 'separate MCP credential required')
    dns = approval['expectedDnsName']
    require(isinstance(dns, str) and re.fullmatch(r'[a-zA-Z0-9.-]+\.ts\.net', dns), 'reviewed MagicDNS required')
    require(approval['expectedTailscaleNodeId'], 'reviewed Tailscale node ID required')
    # Persistent logs are root-only; never replay application output into Fly logs.
    (ROOT/'logs').mkdir(exist_ok=True, mode=0o700)
    private(ROOT/'logs')
    Path('/run/tailscale-private').mkdir(mode=0o700, exist_ok=True)
    private('/run/tailscale-private')
    if not Path('/dev/net/tun').exists():
        Path('/dev/net').mkdir(exist_ok=True)
        os.mknod('/dev/net/tun', stat.S_IFCHR | 0o600, os.makedev(10, 200))
    require(stat.S_ISCHR(Path('/dev/net/tun').stat().st_mode), 'TUN type mismatch')
    # Tailscaled is the only privileged daemon; no auth key, no up/login fallback.
    with open(ROOT/'logs/tailscaled.log', 'ab', buffering=0) as log:
        ts = subprocess.Popen(['/usr/bin/setpriv', '--bounding-set=-all,+net_admin,+net_raw,+net_bind_service',
            '--inh-caps=-all', '--ambient-caps=-all', '--no-new-privs', '--',
            'tailscaled', '--state=/data/tailscale/tailscaled.state',
            '--statedir=/data/tailscale', '--socket=/run/tailscale-private/tailscaled.sock',
            '--tun=tailscale0', '--port=41641'], env={'PATH':PATH, 'HOME':'/root'},
            start_new_session=True, stdin=subprocess.DEVNULL, stdout=log, stderr=log)
    CHILDREN.append(ts)
    status = None
    for _ in range(120):
        require(ts.poll() is None, 'tailscaled exited')
        try:
            status = json.loads(subprocess.check_output(TS+['status', '--json'], env={'PATH':PATH}, stderr=subprocess.DEVNULL))
            if status.get('BackendState') == 'Running':
                break
        except Exception:
            pass
        time.sleep(0.5)
    require(status and status.get('BackendState') == 'Running', 'identity requires intervention; no reauthentication')
    require(status['Self']['DNSName'].rstrip('.') == dns and status['Self']['ID'] == approval['expectedTailscaleNodeId'], 'Tailscale identity mismatch')
    home = '/data/hub/home'
    he = env_for(home)
    he.update(HUB_TOKEN=host, WORKSPACER_PLUGIN_SANDBOX='enforce')
    # The root helper can only inspect/control this node's fixed HTTPS proxy.
    # Its socket is 0600 for the hub UID; workers never see the Tailscale socket.
    network_secret = secrets.token_urlsafe(32)
    network_token = cfg/'network-admin-token'
    fd = os.open(network_token, os.O_WRONLY|os.O_CREAT|os.O_TRUNC|os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as stream:
        stream.write(network_secret)
    os.chown(network_token, 10002, 10002)
    os.chmod(network_token, 0o600)
    network_env = env_for('/root')
    network_env['WKS_NETWORK_ADMIN_TOKEN'] = network_secret
    network_socket = Path('/run/workspacer-network/admin.sock')
    if network_socket.exists():
        require(network_socket.is_socket(), 'unexpected network socket path')
        network_socket.unlink()
    netadmin = spawn(['python3', '/opt/combined/network-admin.py'], network_env, 0,
        '/root', ROOT/'logs/network-admin.log')
    for _ in range(100):
        require(netadmin.poll() is None, 'network helper exited')
        if network_socket.is_socket():
            break
        time.sleep(0.05)
    require(network_socket.is_socket(), 'network helper socket missing')
    os.chown(network_socket, 10002, 10002)
    os.chmod(network_socket, 0o600)
    he.update(WKS_NETWORK_ADMIN_SOCKET=str(network_socket),
        WKS_NETWORK_ADMIN_TOKEN_FILE=str(network_token), WKS_UPLOAD_PROVIDER='worker')
    # Power configuration is root-installed, and its credential is readable
    # only by the hub UID. Worker and agent environments still come from we.
    power = read_json(Path('/opt/combined/power.json'))
    require(power['mode'] in ('observe', 'stop', 'off'), 'invalid power mode')
    require(power['app'] == os.environ.get('FLY_APP_NAME') and power['machine'] == os.environ.get('FLY_MACHINE_ID'), 'power target identity mismatch')
    token_path = Path('/run/workspacer-power-token')
    real(token_path)
    os.chown(token_path, 10002, 10002)
    os.chmod(token_path, 0o600)
    he.update(WKS_MACHINE_POWER='fly', WKS_MACHINE_WAKE='http',
        WKS_MACHINE_WAKE_URL=power['wakeUrl'], WKS_MACHINE_IDLE_MODE=power['mode'],
        WKS_MACHINE_IDLE_TIMEOUT=power['timeout'], FLY_APP_NAME=power['app'],
        FLY_MACHINE_ID=power['machine'], FLY_API_TOKEN_FILE=str(token_path))
    hc = home+'/.config'
    hub = spawn(['hub', '--addr', '127.0.0.1:7895', '--nodes-file', '', '--brain-scope', 'off',
        '--tokens-file', hc+'/workspacer/tokens.json', '--peers-file', hc+'/workspacer/peers.json',
        '--plugins-dir', hc+'/workspacer/plugins', '--webapp-dir', '/usr/local/share/workspacer/web',
        '--layout-file', hc+'/workspacer-hub/layout.json', '--push-dir', hc+'/workspacer-hub',
        '--jobs-file', hc+'/workspacer-hub/jobs.json' if (approval['jobsEnabled'] or approval.get('ownerOnlyJobsEnabled', False)) else '',
        '--examples-dir', '/usr/local/share/workspacer/examples',
        '--trusted-host', dns, '--plugin-origin', 'https://'+dns+':8443'], he, 10002, home, ROOT/'logs/hub.log')
    ready('http://127.0.0.1:7895/health', hub)
    sharing = ROOT/'network-sharing.json'
    sharing_enabled = read_json(sharing).get('enabled') is True if sharing.exists() else True
    if sharing_enabled:
        for port in ('443', '8443'):
            subprocess.run(TS+['serve', '--bg', '--https='+port, 'http://127.0.0.1:7895'], env={'PATH':PATH},
                check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        subprocess.run(TS+['serve', 'reset'], env={'PATH':PATH}, check=True,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    we = env_for('/data/home')
    we.update(PATH='/usr/local/go/bin:/data/go/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
        WKS_NODE_ID='fly-node', XDG_CACHE_HOME='/data/cache/xdg', GOPATH='/data/go', GOMODCACHE='/data/go/pkg/mod',
        GOCACHE='/data/go/cache', BUNDLE_PATH='/data/bundle', BUN_INSTALL_CACHE_DIR='/data/bun',
        npm_config_cache='/data/npm', HISTFILE='/data/home/.bash_history', RUSTUP_HOME='/usr/local/rustup',
        WKS_STATE_DB=str(db), WKS_MCP_FACADE_URL='http://127.0.0.1:7897/mcp', WKS_MCP_UNTOKENED='deny')
    init = spawn(['claudemon', 'init', '--hook-port', '7890'], we, 10001, '/data/home', ROOT/'logs/worker.log')
    require(init.wait(timeout=30) == 0, 'hook init failed')
    CHILDREN.remove(init)
    cl = spawn(['claudemon', 'serve', '--host', '127.0.0.1', '--api-port', '7891', '--hook-port', '7890', '--db-path', str(db)], we, 10001, '/data/home', ROOT/'logs/worker.log')
    ready('http://127.0.0.1:7891/sessions', cl)
    mp = spawn(['mcp', '--addr', '127.0.0.1:7897', '--hub', 'ws://127.0.0.1:7895/bus',
        '--tokens', '/data/home/.config/workspacer/tokens.json', '--untokened', 'deny'],
        dict(we, HUB_TOKEN=mcp['token']), 10001, '/data/home', ROOT/'logs/mcp.log')
    ready('http://127.0.0.1:7897/health', mp)
    spawn(['brain', '--hub', 'ws://127.0.0.1:7895/bus', '--claudemon', 'http://127.0.0.1:7891',
        '--scope', 'full', '--mcp-facade', 'http://127.0.0.1:7897/mcp'], dict(we, HUB_TOKEN=provider['token']),
        10001, '/data/home', ROOT/'logs/brain.log')
    Path('/run/combined-doorbell').mkdir(mode=0o755)
    os.chmod('/run/combined-doorbell', 0o755)  # PID1 umask 077 must not hide the public doorbell.
    Path('/run/combined-doorbell/index.html').write_text('workspacer machine awake\n')
    os.chmod('/run/combined-doorbell/index.html', 0o644)
    Path('/run/combined-doorbell/health').write_text('workspacer machine awake\n')
    os.chmod('/run/combined-doorbell/health', 0o644)
    spawn(['busybox', 'httpd', '-f', '-p', '0.0.0.0:8080', '-h', '/run/combined-doorbell'], we, 10001, '/data/home', ROOT/'logs/doorbell.log')
    print('combined services started; paired-client/provider acceptance remains required', flush=True)
    while True:
        for p in CHILDREN:
            require(p.poll() is None, 'service exited')
        # Popen owns known children; reap only orphan descendants not in that set.
        known = {p.pid for p in CHILDREN}
        for p in Path('/proc').glob('[0-9]*'):
            if int(p.name) not in known:
                try:
                    os.waitpid(int(p.name), os.WNOHANG)
                except (ChildProcessError, ProcessLookupError):
                    pass
        time.sleep(1)

if __name__ == '__main__':
    def terminate(*_):
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, terminate)
    signal.signal(signal.SIGINT, terminate)
    try:
        boot()
    except Exception:
        print('FAIL CLOSED: combined preflight/service failed; inspect root-private evidence', file=sys.stderr)
        sys.exit(1)
    finally:
        stop()
