#!/usr/bin/env python3
"""Exercise the documentation guard against a private repository-shaped fixture."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('check-doc-drift.sh')
DOCUMENTS = ['services/claudemon/README.md', 'services/hub-rs/MIGRATION.md',
             'apps/desktop/README.md', 'apps/tui/README.md']

class DocumentationGuard(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'scripts').mkdir()
        shutil.copyfile(SCRIPT, self.root / 'scripts/check-doc-drift.sh')
        for name in DOCUMENTS:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('Current implementation is available.\n')

    def run_guard(self, strict=False, path=None):
        env = {**os.environ, 'WKS_DOC_DRIFT_STRICT': '1' if strict else '0'}
        if path is not None:
            env['PATH'] = path
        return subprocess.run(['bash', str(self.root / 'scripts/check-doc-drift.sh')],
                              env=env, text=True, capture_output=True)

    def test_clean_without_legacy_directory(self):
        result = self.run_guard()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('No stale maturity phrases', result.stdout)

    def test_each_required_document_must_exist(self):
        for name in DOCUMENTS:
            target = self.root / name
            target.unlink()
            result = self.run_guard()
            self.assertEqual(result.returncode, 2)
            self.assertIn(name, result.stderr)
            self.assertNotIn('No stale', result.stdout)
            target.write_text('Current implementation.\n')

    def test_stale_language_is_informational_or_strict(self):
        (self.root / DOCUMENTS[1]).write_text('This endpoint is not implemented.\n')
        result = self.run_guard()
        self.assertEqual(result.returncode, 0)
        self.assertIn(DOCUMENTS[1], result.stdout)
        self.assertEqual(self.run_guard(strict=True).returncode, 1)

    def test_grep_failure_is_never_clean(self):
        binary = self.root / 'bin'
        binary.mkdir()
        grep = binary / 'grep'
        grep.write_text('#!/bin/sh\nexit 2\n')
        grep.chmod(0o700)
        result = self.run_guard(path=f"{binary}:{os.environ['PATH']}")
        self.assertEqual(result.returncode, 2)
        self.assertIn('scan failed', result.stderr)
        self.assertNotIn('No stale', result.stdout)

if __name__ == '__main__':
    unittest.main()
