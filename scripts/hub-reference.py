#!/usr/bin/env python3
"""Explicit historical Go oracle commands. Ordinary Rust/TS checks do not use this."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SEAL = "0b06d5507b2ab23d2bd63400e0921fbcaa76407029987607acb7bbe1137d4525"
MANIFEST = "tools/capability-source-check/go-reference-provenance.json"
VOCABULARY = "apps/desktop/tests/fixtures/capability-parameter-vocabulary.json"
GO_FIXTURES = [
    (["./cmd/brain"], "^Test(RustMigrationSnapshotFixtures|ContextHealthFormattingMatchesDesktopContract|ContextWatchRejectsUnsupportedProvidersWithoutUsingSlots|CumulativeCodexContractCannotFireContextWatch|TelemetryEpochKeepsAdjacentProductionValuesDistinct|ClaudeProjectDirNameContractCases|HeadlessFileWatch.*)$"),
    (["./internal/bus"], "^TestMigrationBusFixtures$"),
    (["./cmd/mcp"], "^TestRustMigrationToolCatalog$"),
    (["./internal/jobs"], "^TestRustMigrationJobFixtures$"),
    (["./internal/quiescence"], "^TestPortableFleetQuiescenceContract$"),
    (["./internal/routing", "./internal/limits"], "^TestPortableRust(Routing|Pacing)Contract$"),
]
TS_FIXTURES = [
    "src/main/shared/structuredResult.test.ts", "src/main/shared/workerEscalation.test.ts",
    "src/main/shared/fleetMessages.test.ts", "src/main/services/thresholdWatch.test.ts",
]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def run(argv, cwd, *, env=None, capture=False):
    return subprocess.run(argv, cwd=cwd, env=env, check=True,
                          stdout=subprocess.PIPE if capture else None)


def validate(root, selected):
    if not selected:
        raise ValueError("Set WKS_HUB_REFERENCE_ROOT to the explicit pinned historical repository checkout; no current-tree fallback")
    checkout = Path(selected).expanduser().resolve(strict=True)
    manifest_bytes = (root / MANIFEST).read_bytes()
    if digest(manifest_bytes) != SEAL:
        raise ValueError("historical provenance manifest seal changed")
    manifest = json.loads(manifest_bytes)
    capture_bytes = (root / manifest["capturePath"]).read_bytes()
    if digest(capture_bytes) != manifest["captureSha256"]:
        raise ValueError("historical capture digest changed")
    capture = json.loads(capture_bytes)
    if manifest["mode"] != "captured-provenance" or manifest["version"] != 1:
        raise ValueError("unsupported historical provenance mode")
    if sum(len(m["dangerous"]) for m in capture["methods"].values()) != 84:
        raise ValueError("historical binding population changed")
    for revision, expected in [("HEAD", manifest["referenceCommit"]), ("HEAD^{tree}", manifest["referenceTree"])]:
        actual = run(["git", "rev-parse", "--verify", revision], checkout, capture=True).stdout.decode().strip()
        if actual != expected:
            raise ValueError(f"historical checkout {revision} does not match pinned provenance")
    status = run(["git", "status", "--porcelain", "--untracked-files=all"], checkout, capture=True).stdout
    if status:
        raise ValueError("historical checkout must have no modified or untracked nonignored files")
    tracked = run(["git", "ls-files", "--cached", "-z"], checkout, capture=True).stdout.split(b"\0")
    if not any(tracked):
        raise ValueError("historical checkout tracked-file inventory is empty")
    for entry in filter(None, tracked):
        path = checkout / os.fsdecode(entry)
        if not os.path.lexists(path):
            raise ValueError(f"historical tracked command input unavailable: {os.fsdecode(entry)}")
    hashes = dict(capture["sources"])
    hashes["services/hub/cmd/brain/capspec_params_test.go"] = capture["scannerSha256"]
    hashes[VOCABULARY] = capture["vocabularySha256"]
    for path, expected in hashes.items():
        if digest((checkout / path).read_bytes()) != expected:
            raise ValueError(f"historical source digest changed: {path}")
    for path in ["services/hub/go.mod", "services/hub/go.sum", "services/hub/cmd/hub-reference/main.go", "services/hub/scripts/routing-limit-harness.mjs"]:
        if not (checkout / path).is_file():
            raise ValueError(f"historical command input unavailable: {path}")
    return checkout


def invoke(command, root, checkout):
    hub = checkout / "services/hub"
    env = dict(os.environ)
    env["GOFLAGS"] = (env.get("GOFLAGS", "") + " -mod=readonly").strip()
    cargo = ["cargo", "test", "--locked", "--manifest-path", str(root / "services/hub-rs/Cargo.toml")]
    if command == "verify":
        print("Pinned historical checkout and captured source digests verified.")
    elif command == "test":
        run(["go", "test", "-mod=readonly", "-race", "-count=1", "./..."], hub, env=env)
    elif command == "routing-harness":
        env.pop("NO_COLOR", None)
        env["ROUTING_HARNESS_REQUIRE_ROUTING"] = "1"
        run(["node", str(hub / "scripts/routing-limit-harness.mjs")], hub, env=env)
    elif command in ("vocabulary-check", "vocabulary-export"):
        data = run(["go", "run", "-mod=readonly", "./cmd/hub-reference", "--snapshot"], hub, env=env, capture=True).stdout
        if command == "vocabulary-export":
            sys.stdout.buffer.write(data)
        elif data != (root / "services/hub-rs/assets/hub-vocabulary.json").read_bytes():
            raise ValueError("historical vocabulary export differs from retained Rust asset")
    elif command == "parity":
        run(cargo, root, env=env)
        with tempfile.TemporaryDirectory(prefix="wks-hub-reference-") as directory:
            binary = Path(directory) / ("hub-reference.exe" if os.name == "nt" else "hub-reference")
            run(["go", "build", "-mod=readonly", "-o", str(binary), "./cmd/hub-reference"], hub, env=env)
            test_env = dict(env, WKS_GO_HUB_REFERENCE=str(binary))
            run(cargo + ["--test", "compatibility", "shared_contracts_go_reference", "--", "--ignored"], root, env=test_env)
        for packages, pattern in GO_FIXTURES:
            run(["go", "test", "-mod=readonly", *packages, "-run", pattern, "-count=1"], hub, env=env)
        run(["npm", "run", "test:main", "--", *TS_FIXTURES], root / "apps/desktop", env=env)
        invoke("vocabulary-check", root, checkout)
    else:
        raise ValueError(f"unknown reference command: {command}")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["verify", "test", "parity", "routing-harness", "vocabulary-check", "vocabulary-export"])
    args = parser.parse_args(argv)
    try:
        checkout = validate(ROOT, os.environ.get("WKS_HUB_REFERENCE_ROOT"))
        invoke(args.command, ROOT, checkout)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f"hub-reference: {error}", file=sys.stderr)
        return error.returncode if isinstance(error, subprocess.CalledProcessError) and error.returncode > 0 else 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
