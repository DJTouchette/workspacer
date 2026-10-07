#!/usr/bin/env python3
"""Prove real CLIs consume Workspacer's per-session agent skills, via a mock API.

Usage: check-agent-skill-discovery.py --claude /cached/claude --codex /cached/codex

The launcher (agentSkillPlugins.ts) materializes the bundle into a scratch home.
Claude then gets the ordinary plugin through `--plugin-dir` and invoking its
response-card skill must put the skill body into the request. Codex gets the
plugin's skills directory through the app-server's `skills/extraRoots/set`,
exactly as claudemon applies `skill_roots`, and every ordinary skill must be
listed (description and file under that root) in the model-visible context.
(An app-server client only expands a `$skill` mention when it sends an explicit
`skill` input item; Workspacer sends text, so the listing is what an agent
actually gets.) All state stays in a temporary directory
inside this worktree; no model is called.
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


def mock_api(requests):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *unused): pass
        def do_POST(self):
            requests.append(self.rfile.read(int(self.headers['Content-Length'])).decode())
            # A terminal mock error is enough: the recorded request proves
            # what the native CLI actually supplied as model context.
            data = json.dumps({'type': 'error', 'error': {'type': 'invalid_request_error',
                               'message': 'Fixture captured; no model call performed'}}).encode()
            self.send_response(400); self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(data))); self.end_headers(); self.wfile.write(data)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server, 'http://127.0.0.1:' + str(server.server_port)


def codex_turn(binary, project, env, url, roots, prompt):
    command = [str(binary), 'app-server',
               '-c', 'model_provider="fixture"', '-c', 'model_providers.fixture.name="Fixture"',
               '-c', 'model_providers.fixture.base_url="' + url + '/v1"',
               '-c', 'model_providers.fixture.wire_api="responses"',
               '-c', 'model_providers.fixture.requires_openai_auth=false',
               '-c', 'model_providers.fixture.request_max_retries=0',
               '-c', 'model_providers.fixture.stream_max_retries=0', '-c', 'model="fixture"']
    server = subprocess.Popen(command, cwd=project, env=env, stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
    def call(ident, method, params):
        server.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': ident, 'method': method, 'params': params}) + '\n')
        server.stdin.flush()
        while True:
            message = json.loads(server.stdout.readline())
            if message.get('id') == ident:
                return message
    try:
        call(1, 'initialize', {'clientInfo': {'name': 'workspacer-check', 'version': '0'}})
        assert 'error' not in call('workspacer-skill-roots', 'skills/extraRoots/set', {'extraRoots': roots})
        thread = call(2, 'thread/start', {'cwd': str(project)})['result']['thread']['id']
        call(3, 'turn/start', {'threadId': thread, 'input': [{'type': 'text', 'text': prompt}]})
        # The turn fails against the mock; wait for its terminal notification.
        while True:
            message = json.loads(server.stdout.readline())
            if message.get('method') in ('turn/completed', 'error'):
                return
    finally:
        server.kill()


with tempfile.TemporaryDirectory(prefix='.skill-discovery-', dir=desktop) as tmp:
    scratch = Path(tmp)
    bundle = scratch / 'launcher.cjs'
    subprocess.run([str(desktop / 'node_modules/.bin/esbuild'),
        str(desktop / 'src/main/services/agentSkillPlugins.ts'), '--bundle', '--platform=node',
        '--format=cjs', '--outfile=' + str(bundle)], check=True, capture_output=True)
    results = {}
    for provider, binary in [('claude', args.claude), ('codex', args.codex)]:
        project = scratch / provider / 'project'; home = scratch / provider / 'home'
        project.mkdir(parents=True); (home / '.codex').mkdir(parents=True)
        subprocess.run(['git', 'init', '-q', str(project)], check=True)
        env = {key: os.environ[key] for key in ['PATH', 'LANG'] if key in os.environ}
        env.update(HOME=str(home), CODEX_HOME=str(home / '.codex'), CLAUDE_CONFIG_DIR=str(home / '.claude'),
                   XDG_CONFIG_HOME=str(home / '.config'), XDG_CACHE_HOME=str(home / '.cache'),
                   DISABLE_AUTOUPDATER='1', CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1',
                   ANTHROPIC_API_KEY='fixture-only', OPENAI_API_KEY='fixture-only')
        launch = json.loads(subprocess.check_output(
            ['node', '-e', 'const l=require(process.argv[1]).prepareAgentSkills(process.argv[2],process.argv[3],'
             '{home:process.argv[4]});process.stdout.write(JSON.stringify(l))',
             str(bundle), provider, str(project), str(home)], env=env, text=True))
        requests = []
        server, url = mock_api(requests)
        try:
            if provider == 'claude':
                env['ANTHROPIC_BASE_URL'] = url
                subprocess.run([str(binary), '-p', '/workspacer:workspacer-response-cards Make a three-item checklist.',
                                *launch['args'], '--model', 'claude-sonnet-4-6', '--max-turns', '1', '--tools', '',
                                '--strict-mcp-config', '--mcp-config', '{"mcpServers":{}}',
                                '--setting-sources', 'project'],
                               cwd=project, env=env, capture_output=True, text=True, timeout=45)
            else:
                codex_turn(binary, project, env, url, launch['skillRoots'], 'Make a three-item checklist.')
        finally:
            server.shutdown()
        combined = json.dumps(requests)
        if provider == 'claude':
            consumed = '# Response cards' in combined and 'references/schema.md' in combined
        else:
            root = launch['skillRoots'][0]
            consumed = all(
                name in combined for name in ('spawn-agent', 'project-brief', 'scheduled-jobs',
                                              'workspacer-response-cards')
            ) and 'Present a structured answer' in combined and json.dumps(root)[1:-1] in combined
        results[provider] = {
            'requests': len(requests),
            'skillsConsumed': consumed,
            'projectUntouched': not any((project / d).exists() for d in ('.workspacer', '.claude', '.agents')),
            'version': subprocess.check_output([str(binary), '--version'], env=env, text=True).strip(),
        }
    print(json.dumps(results, indent=2))
    assert all(r['skillsConsumed'] for r in results.values()), 'A CLI did not consume the session skills'
    assert all(r['projectUntouched'] for r in results.values()), 'Skills were written into the project'
