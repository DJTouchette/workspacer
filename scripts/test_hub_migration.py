"""Migration reporting stays read-only; stale review batches fail atomically."""
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
    def test_backlog_filters_sources_without_hiding_cutover_or_mutating_evidence(self):
        manifest = {"files": {
            "services/hub/a/one.go": {"status": "pending"},
            "services/hub/a/two_test.go": {"status": "pending"},
            "services/hub/a/done.go": {"status": "ported"},
            "services/hub/b/old.go": {"status": "retired"},
            "services/hub/b/todo.go": {"status": "pending"},
        }, "cutover": {
            "deployment": {"status": "pending", "evidence": ["checkpoint"]},
            "tui": {"status": "verified", "evidence": ["test"]},
        }}
        original = copy.deepcopy(manifest)
        self.assertEqual(migration.backlog(manifest)["pending_files"], 3)
        report = migration.backlog(manifest, "services/hub/a/")
        self.assertEqual(report["pending_files"], 2)
        self.assertEqual(report["packages"], [{
            "path": "services/hub/a", "pending_files": 2,
            "files": ["services/hub/a/one.go", "services/hub/a/two_test.go"],
        }])
        self.assertEqual(report["pending_cutover"], {"deployment": manifest["cutover"]["deployment"]})
        self.assertEqual(manifest, original)
        self.assertEqual(migration.backlog(manifest, "missing/")["packages"], [])

    def test_retained_parameter_fixture_is_inventoried_without_collecting_runtime_json(self):
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            legacy = root / "services/hub"
            fixture = legacy / "internal/capspec/testdata/param-vocabulary.json"
            fixture.parent.mkdir(parents=True)
            fixture.write_text('{"parameters":[]}')
            (legacy / "private-runtime.json").write_text('{"not_source":true}')
            with patch.object(migration, "ROOT", root), patch.object(migration, "LEGACY", legacy):
                self.assertEqual(set(migration.sources()), {fixture.relative_to(root).as_posix()})

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
