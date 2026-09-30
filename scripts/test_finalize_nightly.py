import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('finalize-nightly.py').resolve()
SHA = 'a' * 40
FAKE = r'''#!/usr/bin/env python3
import json, os, pathlib, sys
p = pathlib.Path(os.environ['FAKE_STATE'])
s = json.loads(p.read_text())
a = sys.argv[1:]
endpoint = a[-1]
s['calls'].append(a)
if 'PATCH' in a:
    assert '--input' in a and '-F' not in a
    body = json.loads(pathlib.Path(a[a.index('--input') + 1]).read_text())
    assert body == {'draft': False} and body['draft'] is False
    s['patched'] = True
    if s['mode'] != 'stays-draft': s['release']['draft'] = False
    if s['mode'] == 'wrong-target': s['release']['target_commitish'] = 'b' * 40
    if s['mode'] == 'wrong-tag': s['release']['tag_name'] = 'stable'
    if s['mode'] == 'missing-asset': s['release']['assets'] = []
    if s['mode'] == 'wrong-size': s['release']['assets'][0]['size'] = 99
    if s['mode'] == 'incomplete-asset': s['release']['assets'][0]['state'] = 'new'
    if s['mode'] == 'no-download': s['release']['assets'][0].pop('browser_download_url')
if '/git/ref/' in endpoint:
    result = {'object': {'type': 'tag' if s['mode'] == 'annotated' else 'commit', 'sha': 'b' * 40 if s['mode'] in ['bad-ref', 'annotated'] else 'a' * 40}}
elif '/git/tags/' in endpoint:
    result = {'object': {'type': 'commit', 'sha': 'a' * 40}}
else:
    result = s['release']
p.write_text(json.dumps(s))
print(json.dumps(result))
'''

class FinalizeNightly(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        gh = self.root / 'gh'
        gh.write_text(FAKE)
        gh.chmod(0o700)
        self.asset = self.root / 'Workspacer-Native-Rust-Preview-Setup-fixture-x64.exe'
        self.asset.write_bytes(b'payload')
        self.state = self.root / 'state.json'
        self.env = {**os.environ, 'PATH': f"{self.root}{os.pathsep}{os.environ['PATH']}", 'FAKE_STATE': str(self.state)}

    def run_case(self, mode='ok', *, pre=None, assets=None):
        release = {'id': 17, 'draft': True, 'prerelease': True, 'tag_name': 'nightly',
                   'target_commitish': SHA, 'assets': [{'name': self.asset.name, 'size': 7,
                   'state': 'uploaded', 'browser_download_url': 'https://example.invalid/fixture'}]}
        if pre: release.update(pre)
        self.state.write_text(json.dumps({'mode': mode, 'release': release, 'calls': [], 'patched': False}))
        result = subprocess.run([sys.executable, str(SCRIPT), '--repo', 'owner/repo', '--release-id', '17',
                                 '--sha', SHA, *(assets if assets is not None else [str(self.asset)])],
                                env=self.env, capture_output=True, text=True, timeout=10)
        return result, json.loads(self.state.read_text())

    def test_typed_publish_readback_and_exact_tag(self):
        result, state = self.run_case()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(state['patched'])
        self.assertEqual(len(state['calls']), 4)
        self.assertIn('--input', state['calls'][1])
        self.assertNotIn('PATCH', state['calls'][2])
        self.assertIn('draft=false', result.stdout)

    def test_successful_patch_cannot_mask_unpublished_or_wrong_artifact(self):
        for mode in ['stays-draft', 'wrong-target', 'wrong-tag', 'missing-asset', 'wrong-size', 'incomplete-asset', 'no-download', 'bad-ref']:
            with self.subTest(mode=mode):
                result, state = self.run_case(mode)
                self.assertTrue(state['patched'])
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn('Published nightly release', result.stdout)

    def test_invalid_draft_inventory_refuses_before_mutation(self):
        for pre in [{'assets': []}, {'draft': False}, {'prerelease': False}, {'id': 18}, {'target_commitish': 'main'}]:
            with self.subTest(pre=pre):
                result, state = self.run_case(pre=pre)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(state['patched'])

    def test_local_asset_must_exist_and_names_be_unique(self):
        for assets in [[str(self.root / 'absent')], [str(self.asset), str(self.asset)]]:
            result, state = self.run_case(assets=assets)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(state['calls'], [])

    def test_annotated_ref_must_resolve_to_exact_commit(self):
        result, state = self.run_case('annotated')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(state['calls']), 5)

    def test_workflow_uses_helper_and_preserves_all_assets(self):
        workflow = (SCRIPT.parent.parent / '.github/workflows/release.yml').read_text()
        self.assertIn('scripts/finalize-nightly.py', workflow)
        self.assertIn('"${assets[@]}"', workflow)
        self.assertNotIn('-F draft=false', workflow)
        self.assertIn('Workspacer-Native-Rust-Preview-Setup-*-x64.exe', workflow)

if __name__ == '__main__':
    unittest.main()
