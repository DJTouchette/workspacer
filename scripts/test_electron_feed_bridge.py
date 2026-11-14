#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('electron-feed-bridge.py')
spec = importlib.util.spec_from_file_location('electron_feed_bridge', SCRIPT)
bridge = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bridge)

REPO, TAG = 'owner/repo', 'v0.170.0'
BASE = f'https://github.com/{REPO}/releases/download/{TAG}/'
FEED = '''version: 0.170.0
files:
  - url: Workspacer-Setup-0.170.0.exe
    sha512: abc/def+ghi==
    size: 123
path: Workspacer-Setup-0.170.0.exe
sha512: abc/def+ghi==
releaseDate: '2026-11-12T00:00:00.000Z'
'''


class ElectronFeedBridgeTests(unittest.TestCase):
    def test_only_url_and_path_change_and_resolve_to_existing_assets(self):
        # GitHub stored the installer with dots; the feed names it with dashes.
        result = bridge.rewrite(FEED, REPO, TAG, {'Workspacer.Setup.0.170.0.exe', 'latest.yml'})
        before, after = FEED.splitlines(), result.splitlines()
        self.assertEqual(len(before), len(after))
        changed = [(a, b) for a, b in zip(before, after) if a != b]
        self.assertEqual([b for _, b in changed], [
            f'  - url: {BASE}Workspacer.Setup.0.170.0.exe',
            f'path: {BASE}Workspacer.Setup.0.170.0.exe',
        ])
        self.assertIn('    sha512: abc/def+ghi==', after)
        self.assertIn('version: 0.170.0', after)

    def test_exact_names_quotes_and_line_endings_are_kept(self):
        feed = 'version: 0.170.0\r\nfiles:\r\n  - url: "Workspacer-0.170.0.AppImage"\r\n    sha512: x\r\npath: Workspacer-0.170.0.AppImage\r\n'
        result = bridge.rewrite(feed, REPO, TAG, {'Workspacer-0.170.0.AppImage'})
        self.assertEqual(result.count('\r\n'), feed.count('\r\n'))
        self.assertIn(f'  - url: {BASE}Workspacer-0.170.0.AppImage\r\n', result)
        # Running it again over its own output changes nothing.
        self.assertEqual(bridge.rewrite(result, REPO, TAG, {'Workspacer-0.170.0.AppImage'}), result)

    def test_missing_or_foreign_targets_are_refused(self):
        with self.assertRaisesRegex(ValueError, 'no asset'):
            bridge.rewrite(FEED, REPO, TAG, {'latest.yml'})
        with self.assertRaisesRegex(ValueError, 'elsewhere'):
            bridge.rewrite(FEED.replace('Workspacer-Setup-0.170.0.exe', 'https://example.invalid/x.exe'),
                           REPO, TAG, {'x.exe'})
        with self.assertRaisesRegex(ValueError, 'no url or path'):
            bridge.rewrite('version: 0.170.0\n', REPO, TAG, set())

    def test_command_skips_a_feed_it_cannot_resolve_and_still_succeeds(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'latest.yml').write_text(FEED)
            (root / 'latest-linux.yml').write_text(FEED.replace('Workspacer-Setup-0.170.0.exe', 'Gone.AppImage'))
            (root / 'assets.json').write_text(json.dumps({'assets': [{'name': 'Workspacer.Setup.0.170.0.exe'}]}))
            result = subprocess.run(
                [sys.executable, str(SCRIPT), '--repo', REPO, '--tag', TAG, '--assets', str(root / 'assets.json'),
                 '--out', str(root / 'out'), str(root / 'latest.yml'), str(root / 'latest-linux.yml')],
                capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual([p.name for p in (root / 'out').iterdir()], ['latest.yml'])
            self.assertIn('::warning::', result.stdout)
            self.assertIn(BASE, (root / 'out/latest.yml').read_text())

    def test_release_workflow_carries_both_feeds(self):
        workflow = (SCRIPT.parent.parent / '.github/workflows/release.yml').read_text()
        self.assertIn('scripts/electron-feed-bridge.py', workflow)
        self.assertIn("gh release download v0.170.0", workflow)
        self.assertIn("--pattern 'latest*.yml'", workflow)


if __name__ == '__main__':
    unittest.main()
