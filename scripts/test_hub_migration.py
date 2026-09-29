"""Deletion evidence must reject stale or incomplete review batches atomically."""
import copy
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("hub_migration", Path(__file__).with_name("hub-migration.py"))
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)


class ReviewTests(unittest.TestCase):
    def test_stale_or_retiring_evidence_never_partially_applies_a_batch(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            legacy = root / "services/hub"
            legacy.mkdir(parents=True)
            (legacy / "old.go").write_text("old")
            (root / "new.rs").write_text("new")
            live = {"services/hub/a.go": "a", "services/hub/b.go": "b"}
            row = {"sha256": "a", "replacement": ["new.rs"], "tests": ["new.rs"]}
            original = {"files": {"existing": {"status": "pending"}}}
            for invalid in [
                {**row, "sha256": "stale"},
                {**row, "sha256": "b", "tests": ["services/hub/old.go"]},
                {**row, "sha256": "b", "status": "retired"},
            ]:
                manifest = copy.deepcopy(original)
                with patch.object(migration, "ROOT", root), patch.object(migration, "LEGACY", legacy):
                    with self.assertRaises(ValueError):
                        migration.apply_review(manifest, {"records": {"services/hub/a.go": row, "services/hub/b.go": invalid}}, live)
                self.assertEqual(manifest, original)

    def test_review_preserves_explicit_architectural_retirement(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            (root / "owner.rs").write_text("owned lifecycle")
            manifest = {"files": {}}
            row = {"sha256": "current", "status": "retired", "reason": "Process supervision is replaced by owned library shutdown.", "replacement": ["owner.rs"], "tests": ["owner.rs"]}
            with patch.object(migration, "ROOT", root), patch.object(migration, "LEGACY", root / "old"):
                migration.apply_review(manifest, {"records": {"old/source.go": row}}, {"old/source.go": "current"})
            self.assertEqual(manifest["files"]["old/source.go"], row)


if __name__ == "__main__":
    unittest.main()
