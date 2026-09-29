#!/usr/bin/env python3
"""Replace only reviewed isolated runtime spawn blocks; preserve root/UID guards."""
import ast
from pathlib import Path
import sys

MARKER = '# Rust backend roles v1: hub UID10002, worker UID10001.'
HUB = '''    # Rust backend roles v1: hub UID10002, worker UID10001.
    hub_args = ['/usr/local/bin/workspacer-rust', 'serve', '--hub-only', '--quiet', '--no-mcp',
        '--host', '127.0.0.1', '--hub-port', '7895', '--nodes-file', '',
        '--config-dir', hc+'/workspacer', '--data-dir', hc+'/workspacer-hub',
        '--tokens-file', hc+'/workspacer/tokens.json', '--peers-file', hc+'/workspacer/peers.json',
        '--plugins-dir', hc+'/workspacer/plugins', '--webapp-dir', '/usr/local/share/workspacer/web',
        '--push-dir', hc+'/workspacer-hub', '--examples-dir', '/usr/local/share/workspacer/examples',
        '--uploads-to-worker', '--trusted-host', dns, '--plugin-origin', 'https://'+dns+':8443']
    if approval['jobsEnabled'] or approval.get('ownerOnlyJobsEnabled', False):
        hub_args += ['--jobs-file', hc+'/workspacer-hub/jobs.json']
    else:
        hub_args += ['--no-jobs']
    hub = spawn(hub_args, he, 10002, home, ROOT/'logs/hub.log')
'''
WORKER = '''    # One owned engine/MCP/provider graph, with the original pristine worker env.
    # Fly/network-admin credentials stay exclusively in he, never in this dict.
    worker = spawn(['/usr/local/bin/workspacer-rust', 'serve', '--quiet', '--host', '127.0.0.1',
        '--hub-port', '0', '--upstream', 'ws://127.0.0.1:7895/bus', '--provider-scope', 'full',
        '--node-id', 'fly-node', '--config-dir', '/data/home/.config/workspacer',
        '--home-dir', '/data/home', '--claudemon-db-path', str(db),
        '--claudemon-api-port', '7891', '--claudemon-hook-port', '7890', '--mcp-port', '7897'],
        dict(we, HUB_TOKEN=provider['token'], WKS_MCP_HUB_TOKEN=mcp['token']),
        10001, '/data/home', ROOT/'logs/worker.log')
    ready('http://127.0.0.1:7891/sessions', worker)
    ready('http://127.0.0.1:7897/health', worker)
'''
CREDENTIAL = '''    require(approval['mcpScope'] == 'operator' and mcp.get('facadeAuthority') is True,
        'Rust full worker needs an explicitly granted operator facade credential')
    worker_identity = DATA/'home/.config/workspacer/remote-token'
    info = real(worker_identity)
    require(info.st_uid == 10001 and info.st_gid == 10001 and not info.st_mode & 0o077,
        'initialize the private Rust worker identity as its owner before cutover')
    local_host = worker_identity.read_text().strip()
    require(local_host and local_host not in (host, provider['token'], mcp['token']),
        'worker local identity must be distinct; no credential reuse or automatic mint')
'''

def executable(node):
    call = node.value if isinstance(node, (ast.Assign, ast.Expr)) else None
    if not isinstance(call, ast.Call) or not isinstance(call.func, ast.Name) or call.func.id != 'spawn':
        return None
    args = call.args[0]
    if isinstance(args, ast.List) and args.elts and isinstance(args.elts[0], ast.Constant):
        return args.elts[0].value
    return None

def transform(source):
    tree = ast.parse(source)
    if MARKER in source:
        if any(executable(node) in ('hub', 'brain', 'mcp') for node in ast.walk(tree)):
            raise ValueError('mixed Rust/legacy supervisor refused')
        return source
    for guard in ("os.chown(token_path, 10002, 10002)", "WKS_NETWORK_ADMIN_SOCKET", 'mount_guard()',
                  'private_proc()', "approval['manifestSha256']", "approval['expectedTailscaleNodeId']"):
        if guard not in source:
            raise ValueError('unrecognized isolated supervisor guard: '+guard)
    boot = next(node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name == 'boot')
    hubs = [node for node in boot.body if executable(node) == 'hub']
    if len(hubs) != 1 or hubs[0].value.args[2].value != 10002:
        raise ValueError('unrecognized hub owner')
    hub = hubs[0]
    if not isinstance(hub, ast.Assign) or [v.id for v in hub.targets] != ['hub']:
        raise ValueError('unrecognized hub process variable')
    start = next(i for i,node in enumerate(boot.body) if executable(node) == 'claudemon')
    end = next(i for i,node in enumerate(boot.body) if executable(node) == 'brain')
    nodes = boot.body[start:end+1]
    if len(nodes) != 8 or [executable(node) for node in nodes] != ['claudemon',None,None,'claudemon',None,'mcp',None,'brain']:
        raise ValueError('worker block changed; refusing to erase unknown policy')
    for node in nodes:
        if executable(node) and node.value.args[2].value != 10001:
            raise ValueError('unrecognized worker owner')
    reference = ast.parse((Path(__file__).parent/'fixtures/isolated-supervisor.py').read_text())
    reference_boot = next(n for n in reference.body if isinstance(n,ast.FunctionDef) and n.name=='boot')
    reference_hub = next(n for n in reference_boot.body if executable(n)=='hub')
    first = next(i for i,n in enumerate(reference_boot.body) if executable(n)=='claudemon')
    last = next(i for i,n in enumerate(reference_boot.body) if executable(n)=='brain')
    if ast.dump(hub,include_attributes=False) != ast.dump(reference_hub,include_attributes=False) or \
       [ast.dump(n,include_attributes=False) for n in nodes] != [ast.dump(n,include_attributes=False) for n in reference_boot.body[first:last+1]]:
        raise ValueError('spawn policy differs from the reviewed fixture; preserve and review it explicitly')
    anchor = "    require(mcp['token'] != provider['token'], 'separate MCP credential required')\n"
    if source.count(anchor) != 1:
        raise ValueError('credential separation guard changed')
    lines = source.splitlines(keepends=True)
    edits = [(hub.lineno-1,hub.end_lineno,HUB),(nodes[0].lineno-1,nodes[-1].end_lineno,WORKER)]
    for first,last,replacement in sorted(edits,reverse=True):
        lines[first:last] = [replacement]
    result = ''.join(lines).replace(anchor,anchor+CREDENTIAL)
    # No blanket env copying, manifest rewrite or topology change is introduced.
    ast.parse(result)
    return result

if __name__ == '__main__':
    if len(sys.argv) != 3:
        raise SystemExit('usage: upgrade-supervisor.py INPUT OUTPUT (staged files only)')
    source = Path(sys.argv[1]).read_text()
    result = transform(source)
    Path(sys.argv[2]).write_text(result)
