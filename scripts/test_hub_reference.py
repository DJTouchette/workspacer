"""Wrapper tests are source-only: no Git, Go, Node or Cargo process executes."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import unittest
import tempfile
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("hub_reference", Path(__file__).with_name("hub-reference.py"))
ref = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ref)


class ReferenceCommands(unittest.TestCase):
    @contextlib.contextmanager
    def historical_files(self):
        # Synthetic historical I/O keeps routine unit tests portable after Go deletion.
        capture=json.loads((ref.ROOT / "tools/capability-source-check/go-reference.json").read_text())
        hashes=dict(capture["sources"])
        hashes["services/hub/cmd/brain/capspec_params_test.go"]=capture["scannerSha256"]
        hashes[ref.VOCABULARY]=capture["vocabularySha256"]
        real_read=Path.read_bytes
        real_digest=ref.digest
        with tempfile.TemporaryDirectory(prefix="wks-reference-wrapper-test-") as directory:
            self.checkout=Path(directory)
            for name in ["services/hub/go.mod","services/hub/go.sum","services/hub/cmd/hub-reference/main.go","services/hub/scripts/routing-limit-harness.mjs"]:
                path=self.checkout / name;path.parent.mkdir(parents=True,exist_ok=True);path.write_text("fixture")
            def read(path):
                if path.is_relative_to(self.checkout) and path.relative_to(self.checkout).as_posix() in hashes:
                    return b"test-captured-digest:" + hashes[path.relative_to(self.checkout).as_posix()].encode()
                return real_read(path)
            def digest(data):
                return data.removeprefix(b"test-captured-digest:").decode() if data.startswith(b"test-captured-digest:") else real_digest(data)
            with patch.object(Path,"read_bytes",read), patch.object(ref,"digest",digest):
                yield self.checkout

    def git(self, argv, cwd, **kwargs):
        manifest = json.loads((ref.ROOT / ref.MANIFEST).read_text())
        self.assertEqual(cwd, self.checkout)
        self.assertEqual(argv[0], "git")
        if argv[1] == "status":
            return subprocess.CompletedProcess(argv, 0, b"")
        if argv[1] == "ls-files":
            return subprocess.CompletedProcess(argv, 0, b"services/hub/go.mod\0services/hub/go.sum\0")
        return subprocess.CompletedProcess(argv, 0, (manifest["referenceTree"] if argv[-1] == "HEAD^{tree}" else manifest["referenceCommit"]).encode())

    def test_checked_subprocess_preserves_argv_cwd_environment_and_failure(self):
        with patch.object(ref.subprocess,"run",side_effect=subprocess.CalledProcessError(9,["go","test"])) as process:
            with self.assertRaises(subprocess.CalledProcessError):
                ref.run(["go","test"],Path("/historical"),env={"GOFLAGS":"-mod=readonly"},capture=True)
            process.assert_called_once_with(["go","test"],cwd=Path("/historical"),env={"GOFLAGS":"-mod=readonly"},check=True,stdout=subprocess.PIPE)

    def test_wrapper_and_rust_guard_pin_the_same_manifest_seal(self):
        source=(ref.ROOT / "tools/capability-source-check/src/reference.rs").read_text()
        self.assertIn(f'const SEAL: &str = "{ref.SEAL}";',source)

    def test_missing_opt_in_fails_before_any_process(self):
        with patch.object(ref, "run") as run:
            with self.assertRaisesRegex(ValueError, "WKS_HUB_REFERENCE_ROOT"):
                ref.validate(ref.ROOT, None)
            run.assert_not_called()

    def test_pinned_clean_checkout_validates_captured_originals(self):
        with self.historical_files() as checkout, patch.object(ref, "run", side_effect=self.git) as run:
            self.assertEqual(ref.validate(ref.ROOT, str(checkout)), checkout)
        self.assertEqual(len(run.call_args_list), 4)
        self.assertEqual(run.call_args_list[-2].args[0], ["git", "status", "--porcelain", "--untracked-files=all"])

    def test_wrong_revision_or_dirty_checkout_refused(self):
        with self.historical_files() as checkout, patch.object(ref, "run", return_value=subprocess.CompletedProcess([], 0, b"wrong")):
            with self.assertRaisesRegex(ValueError, "pinned provenance"):
                ref.validate(ref.ROOT, str(checkout))
        def dirty(argv, cwd, **kwargs):
            return subprocess.CompletedProcess(argv, 0, b" M source.go") if argv[1] == "status" else self.git(argv, cwd, **kwargs)
        with self.historical_files() as checkout, patch.object(ref, "run", side_effect=dirty):
            with self.assertRaisesRegex(ValueError, "modified or untracked"):
                ref.validate(ref.ROOT, str(checkout))

    def test_corrupt_seal_and_missing_source_refused_before_go(self):
        for victim, error in [(ref.MANIFEST, ValueError), ("services/hub/cmd/brain/capspec_params_test.go", FileNotFoundError)]:
            with self.historical_files() as checkout:
                read=Path.read_bytes
                def altered(path):
                    if str(path).endswith(victim):
                        if victim == ref.MANIFEST:
                            return b"{}"
                        raise FileNotFoundError(victim)
                    return read(path)
                with patch.object(Path, "read_bytes", altered), patch.object(ref, "run", side_effect=self.git) as run:
                    with self.assertRaises(error):
                        ref.validate(ref.ROOT, str(checkout))
                    self.assertTrue(all(c.args[0][0] == "git" for c in run.call_args_list))

    def test_missing_tracked_fixture_fails_before_oracle(self):
        def missing(argv,cwd,**kwargs):
            if argv[1]=="ls-files":
                return subprocess.CompletedProcess(argv,0,b"contracts/missing-fixture.json\0")
            return self.git(argv,cwd,**kwargs)
        with self.historical_files() as checkout, patch.object(ref,"run",side_effect=missing) as run:
            with self.assertRaisesRegex(ValueError,"tracked command input unavailable"):
                ref.validate(ref.ROOT,str(checkout))
            self.assertTrue(all(c.args[0][0]=="git" for c in run.call_args_list))

    def test_go_test_exact_command_and_historical_cwd(self):
        old = Path("/historical checkout")
        with patch.object(ref, "run") as run:
            ref.invoke("test", ref.ROOT, old)
        self.assertEqual(run.call_args.args, (["go", "test", "-mod=readonly", "-race", "-count=1", "./..."], old / "services/hub"))
        self.assertTrue(run.call_args.kwargs["env"]["GOFLAGS"].endswith("-mod=readonly"))

    def test_parity_preserves_every_fixture_and_temporary_oracle(self):
        old = Path("/historical checkout")
        exported = (ref.ROOT / "services/hub-rs/assets/hub-vocabulary.json").read_bytes()
        with patch.object(ref, "run", return_value=subprocess.CompletedProcess([], 0, exported)) as run:
            ref.invoke("parity", ref.ROOT, old)
        calls = run.call_args_list
        self.assertEqual(len(calls), 11)
        self.assertEqual(calls[0].args[0], ["cargo", "test", "--locked", "--manifest-path", str(ref.ROOT / "services/hub-rs/Cargo.toml")])
        build = calls[1].args[0]
        self.assertEqual(build[:4], ["go", "build", "-mod=readonly", "-o"])
        self.assertFalse(Path(build[4]).is_relative_to(old))
        self.assertFalse(Path(build[4]).parent.exists(), "temporary oracle should be cleaned")
        self.assertEqual(calls[2].kwargs["env"]["WKS_GO_HUB_REFERENCE"], build[4])
        self.assertEqual(calls[2].args[0][-5:], ["--test", "compatibility", "shared_contracts_go_reference", "--", "--ignored"])
        expected = [
            (["./cmd/brain"], "^Test(RustMigrationSnapshotFixtures|ContextHealthFormattingMatchesDesktopContract|ContextWatchRejectsUnsupportedProvidersWithoutUsingSlots|CumulativeCodexContractCannotFireContextWatch|TelemetryEpochKeepsAdjacentProductionValuesDistinct|ClaudeProjectDirNameContractCases|HeadlessFileWatch.*)$"),
            (["./internal/bus"], "^TestMigrationBusFixtures$"),
            (["./cmd/mcp"], "^TestRustMigrationToolCatalog$"),
            (["./internal/jobs"], "^TestRustMigrationJobFixtures$"),
            (["./internal/quiescence"], "^TestPortableFleetQuiescenceContract$"),
            (["./internal/routing", "./internal/limits"], "^TestPortableRust(Routing|Pacing)Contract$"),
        ]
        for call, (packages, pattern) in zip(calls[3:9], expected):
            self.assertEqual(call.args, (["go", "test", "-mod=readonly", *packages, "-run", pattern, "-count=1"], old / "services/hub"))
        self.assertEqual(calls[9].args, (["npm", "run", "test:main", "--", "src/main/shared/structuredResult.test.ts", "src/main/shared/workerEscalation.test.ts", "src/main/shared/fleetMessages.test.ts", "src/main/services/thresholdWatch.test.ts"], ref.ROOT / "apps/desktop"))
        self.assertEqual(calls[10].args[0], ["go", "run", "-mod=readonly", "./cmd/hub-reference", "--snapshot"])

    def test_routing_harness_uses_historical_script_and_cannot_park(self):
        with patch.dict(ref.os.environ, {"NO_COLOR": "1"}), patch.object(ref, "run") as run:
            ref.invoke("routing-harness", ref.ROOT, Path("/historical"))
        self.assertEqual(run.call_args.args, (["node", "/historical/services/hub/scripts/routing-limit-harness.mjs"], Path("/historical/services/hub")))
        self.assertNotIn("NO_COLOR", run.call_args.kwargs["env"])
        self.assertEqual(run.call_args.kwargs["env"]["ROUTING_HARNESS_REQUIRE_ROUTING"], "1")

    def test_vocabulary_mismatch_fails_without_writing_fixture(self):
        with patch.object(ref, "run", return_value=subprocess.CompletedProcess([], 0, b"changed")), patch.object(Path, "write_bytes") as write:
            with self.assertRaisesRegex(ValueError, "differs"):
                ref.invoke("vocabulary-check", ref.ROOT, ref.ROOT)
            write.assert_not_called()

    def test_validation_precedes_invocation_and_preserves_nonzero_failure(self):
        with patch.object(ref, "validate", side_effect=ValueError("bad root")), patch.object(ref, "invoke") as invoke, contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(ref.main(["parity"]), 1)
            invoke.assert_not_called()
        with patch.object(ref, "validate", return_value=ref.ROOT), patch.object(ref, "invoke", side_effect=subprocess.CalledProcessError(7, ["go", "test"])), contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(ref.main(["test"]), 7)

    def test_make_and_npm_optional_commands_delegate_only_to_wrapper(self):
        make = (ref.ROOT / "Makefile").read_text()
        for target, command in [("test-hub-reference", "test"), ("test-hub-parity", "parity"), ("test-routing-harness", "routing-harness"), ("hub-vocabulary", "vocabulary-check")]:
            self.assertIn(f"{target}:\n\tpython3 scripts/hub-reference.py {command}\n", make)
        package = json.loads((ref.ROOT / "apps/desktop/package.json").read_text())
        self.assertEqual(package["scripts"]["test:hub-reference"], "python3 ../../scripts/hub-reference.py test")
        self.assertIn("cargo test --locked", package["scripts"]["test:hub"])


if __name__ == "__main__":
    unittest.main()
