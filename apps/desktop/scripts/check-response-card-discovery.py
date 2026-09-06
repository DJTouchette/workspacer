#!/usr/bin/env python3
"""Prove native project skill consumption using cached ELF CLIs and a mock API.
Usage: check-response-card-discovery.py --claude /cached/claude --codex /cached/codex
All generated state stays in a temporary project inside this worktree.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading

parser = argparse.ArgumentParser()
parser.add_argument('--claude', required=True, type=Path)
parser.add_argument('--codex', required=True, type=Path)
args = parser.parse_args()
desktop = Path(__file__).resolve().parents[1]
for binary in [args.claude, args.codex]:
    with binary.open('rb') as stream:
        assert stream.read(4) == b'\x7fELF', 'Pass a cached native ELF binary, never an update wrapper'
with tempfile.TemporaryDirectory(prefix='.card-discovery-', dir=desktop) as tmp:
    scratch = Path(tmp)
    bundle = scratch / 'install.cjs'
    subprocess.run([str(desktop / 'node_modules/.bin/esbuild'),
        str(desktop / 'src/main/services/responseCardSkill.ts'), '--bundle', '--platform=node',
        '--format=cjs', '--outfile=' + str(bundle)], check=True, capture_output=True)
    results = {}
    for provider, binary, native_root in [('claude', args.claude, '.claude'), ('codex', args.codex, '.agents')]:
        project = scratch / provider / 'project'; home = scratch / provider / 'home'
        project.mkdir(parents=True); home.mkdir(); (home / '.codex').mkdir()
        subprocess.run(['git', 'init', '-q', str(project)], check=True)
        env = {key: os.environ[key] for key in ['PATH', 'LANG'] if key in os.environ}
        env.update(HOME=str(home), CODEX_HOME=str(home / '.codex'), CLAUDE_CONFIG_DIR=str(home / '.claude'),
                   XDG_CONFIG_HOME=str(home / '.config'), XDG_CACHE_HOME=str(home / '.cache'),
                   DISABLE_AUTOUPDATER='1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1',
                   ANTHROPIC_API_KEY='fixture-only', OPENAI_API_KEY='fixture-only')
        subprocess.run(['node', '-e', 'require(process.argv[1]).installResponseCardSkill(process.argv[2],process.argv[3])',
                        str(bundle), provider, str(project)], env=env, check=True)
        assert (project / native_root / 'skills/workspacer-response-cards/SKILL.md').exists()
        requests = []
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *unused): pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                requests.append(body)
                # A terminal mock error is enough: the recorded request proves
                # what the native CLI actually supplied as model context.
                data = json.dumps({'error': {'type':'invalid_request_error','message':'Fixture captured; no model call performed'}}).encode()
                self.send_response(400); self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        url = 'http://127.0.0.1:' + str(server.server_port)
        if provider == 'claude':
            env['ANTHROPIC_BASE_URL'] = url
            command = [str(binary), '-p', '/workspacer-response-cards Make a three-item checklist.',
                       '--model', 'claude-sonnet-4-6', '--max-turns', '1', '--tools', '',
                       '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}', '--setting-sources', 'project']
        else:
            command = [str(binary), 'exec', '--skip-git-repo-check', '--ephemeral', '-s', 'read-only',
                       '-c', 'model_provider="fixture"', '-c', 'model_providers.fixture.name="Fixture"',
                       '-c', 'model_providers.fixture.base_url="'+url+'/v1"',
                       '-c', 'model_providers.fixture.wire_api="responses"',
                       '-c', 'model_providers.fixture.requires_openai_auth=false',
                       '-m', 'fixture', '$workspacer-response-cards Make a three-item checklist.']
        try:
            run = subprocess.run(command, cwd=project, env=env, capture_output=True, text=True, timeout=45)
        finally: server.shutdown()
        combined = json.dumps(requests)
        consumed = '# Response cards' in combined and 'references/schema.md' in combined
        results[provider] = {'exitCode':run.returncode, 'requests':len(requests), 'skillBodyConsumed':consumed,
                             'version':subprocess.check_output([str(binary),'--version'], env=env, text=True).strip()}
        if not consumed: print(run.stderr[-3000:])
    print(json.dumps(results, indent=2))
    assert all(value['skillBodyConsumed'] for value in results.values()), 'Native CLI did not consume the installed skill body'
