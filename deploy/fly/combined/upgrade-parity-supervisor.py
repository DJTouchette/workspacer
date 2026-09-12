#!/usr/bin/env python3
"""Extend the existing isolated supervisor without changing its identity boundary."""
from pathlib import Path
import sys

path = Path(sys.argv[1])
source = path.read_text()
source = source.replace('import signal\n', 'import signal\nimport secrets\n', 1)
assert "os.chown(token_path, 10002, 10002)" in source, 'hub-only power isolation missing'
assert "dict(we, HUB_TOKEN=mcp['token']), 10001" in source, 'separate worker MCP credential missing'
assert "--nodes-file', ''" in source, 'unexpected node supervision policy'
assert 'WKS_NETWORK_ADMIN_SOCKET' not in source, 'network adapter already installed'
anchor = "    he.update(HUB_TOKEN=host, WORKSPACER_PLUGIN_SANDBOX='enforce')\n"
assert source.count(anchor) == 1, 'unrecognized hub environment'
source = source.replace(anchor, anchor + '''    # The root helper can only inspect/control this node's fixed HTTPS proxy.
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
''')
anchor = "        '--jobs-file', hc+'/workspacer-hub/jobs.json' if approval['jobsEnabled'] else '',"
assert source.count(anchor) == 1, 'unrecognized jobs configuration'
# Job administration is now authenticated-owner-only in the hub, including shell
# jobs. The worker's operator MCP credential cannot enter this control plane.
source = source.replace(anchor, anchor.replace("approval['jobsEnabled']", "(approval['jobsEnabled'] or approval.get('ownerOnlyJobsEnabled', False))") + "\n        '--examples-dir', '/usr/local/share/workspacer/examples',")
anchor = "    for port in ('443', '8443'):\n        subprocess.run(TS+['serve', '--bg', '--https='+port, 'http://127.0.0.1:7895'], env={'PATH':PATH},\n            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)"
assert source.count(anchor) == 1, 'unrecognized HTTPS startup policy'
source = source.replace(anchor, '''    sharing = ROOT/'network-sharing.json'
    sharing_enabled = read_json(sharing).get('enabled') is True if sharing.exists() else True
    if sharing_enabled:
        for port in ('443', '8443'):
            subprocess.run(TS+['serve', '--bg', '--https='+port, 'http://127.0.0.1:7895'], env={'PATH':PATH},
                check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        subprocess.run(TS+['serve', 'reset'], env={'PATH':PATH}, check=True,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)''')
path.write_text(source)
