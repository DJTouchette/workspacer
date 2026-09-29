#!/usr/bin/env python3
"""Inventory the retiring backend and enforce its migration completion gate.

Run `python3 scripts/hub-migration.py refresh` when legacy source changes. This
retains reviewed records and makes new files explicitly pending. No deletion is
performed by this command. `ready` fails until every inventoried source has a
replacement and every test has a portable regression counterpart.
"""
import argparse
import hashlib
import json
import os
import tempfile
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "services/hub-rs/migration.json"
LEGACY = ROOT / "services/hub"


def sources():
    files = {
        path.relative_to(ROOT).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(LEGACY.rglob("*.go"))
        if "cmd/hub-reference/" not in path.relative_to(LEGACY).as_posix()
    }
    # Embedded policy/default data and retained contract fixtures need explicit replacements. Do not
    # glob arbitrary JSON/YAML: a developer may have private runtime state here.
    for relative in (
        "internal/routing/routing.default.yaml",
        "cmd/brain/config_defaults.json",
        "internal/capspec/testdata/param-vocabulary.json",
    ):
        path = LEGACY / relative
        if path.is_file():
            files[path.relative_to(ROOT).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return files


def read():
    if not MANIFEST.exists():
        return {"version": 1, "files": {}, "cutover": {
            name: {"status": "pending", "evidence": []} for name in [
                "native-embedded", "standalone-service", "electron-and-web",
                "tui", "mcp-and-plugins", "federation-and-remote-workers",
                "persisted-state-upgrade", "node-companion-replacement",
                "windows-package", "macos-package", "linux-package",
                "deployment", "ci-and-generators", "legacy-deletion",
            ]}}
    return json.loads(MANIFEST.read_text())


def write(manifest):
    with tempfile.NamedTemporaryFile(mode="w", dir=MANIFEST.parent, delete=False) as target:
        temporary = Path(target.name)
        try:
            target.write(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
            target.flush()
            os.fsync(target.fileno())
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    try:
        temporary.replace(MANIFEST)
    finally:
        temporary.unlink(missing_ok=True)


def apply_review(manifest, plan, live):
    """Validate a complete reviewed batch before changing any recorded evidence."""
    proposed = plan.get("records")
    if not isinstance(proposed, dict) or not proposed:
        raise ValueError("review plan requires nonempty records")
    updates = {}
    for source, row in proposed.items():
        if source not in live or row.get("sha256") != live[source]:
            raise ValueError(f"reviewed source is missing or changed: {source}")
        status = row.get("status", "ported")
        if status not in ("ported", "retired"):
            raise ValueError(f"invalid review status: {source}")
        if status == "retired" and not row.get("reason", "").strip():
            raise ValueError(f"architectural retirement requires a reason: {source}")
        for key in ("replacement", "tests"):
            paths = row.get(key)
            if not isinstance(paths, list) or not paths:
                raise ValueError(f"missing {key} evidence: {source}")
            for path in paths:
                full = (ROOT / path).resolve()
                if not full.is_relative_to(ROOT.resolve()) or not full.is_file() or full.is_relative_to(LEGACY.resolve()):
                    raise ValueError(f"evidence must exist outside the retiring implementation: {path}")
        updates[source] = {**row, "status": status}
    manifest["files"].update(updates)


def backlog(manifest, source_prefix=""):
    """Report ledger work by legacy package, without claiming unported behavior."""
    groups = defaultdict(list)
    for source, row in sorted(manifest["files"].items()):
        if row.get("status") == "pending" and source.startswith(source_prefix):
            groups[Path(source).parent.as_posix()].append(source)
    return {
        "pending_files": sum(len(paths) for paths in groups.values()),
        "packages": [
            {"path": package, "pending_files": len(paths), "files": paths}
            for package, paths in sorted(groups.items(), key=lambda item: (-len(item[1]), item[0]))
        ],
        "pending_cutover": {
            name: row for name, row in sorted(manifest.get("cutover", {}).items())
            if row.get("status") != "verified"
        },
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["refresh", "status", "backlog", "check", "ready", "record", "record-review"])
    parser.add_argument("--json", action="store_true", help="Emit machine-readable backlog (backlog only)")
    parser.add_argument("--prefix", default="", help="Filter pending source paths (backlog only)")
    parser.add_argument("--plan", type=Path, help="Reviewed per-file JSON evidence with current source hashes")
    parser.add_argument("--source")
    parser.add_argument("--replacement", action="append", default=[])
    parser.add_argument("--test", action="append", default=[])
    parser.add_argument("--retire", action="store_true", help="Record an architectural responsibility removed by the new ownership model")
    parser.add_argument("--reason", help="Required explanation for architectural retirement")
    args = parser.parse_args()
    if args.command != "backlog" and (args.json or args.prefix):
        parser.error("--json and --prefix require backlog")
    manifest = read()
    if args.command == "backlog":
        report = backlog(manifest, args.prefix)
        if args.json:
            print(json.dumps(report, indent=2))
        else:
            print(f"Pending ledger entries: {report['pending_files']} (not a count of missing implementations)")
            for package in report["packages"]:
                print(f"{package['pending_files']:4}  {package['path']}")
            print("Pending cutover gates: " + ", ".join(report["pending_cutover"]))
        return 0
    live = sources()
    records = manifest["files"]
    if args.command == "record-review":
        if args.plan is None:
            parser.error("record-review requires --plan")
        try:
            apply_review(manifest, json.loads(args.plan.read_text()), live)
        except (ValueError, OSError, TypeError) as error:
            parser.error(str(error))
        write(manifest)
    if args.command == "record":
        if args.source not in live or not args.replacement or not args.test:
            parser.error("record requires an existing --source, --replacement and --test evidence")
        if args.retire and not (args.reason and args.reason.strip()):
            parser.error("architectural retirement requires --reason and replacement ownership tests")
        for path in args.replacement + args.test:
            if not (ROOT / path).is_file() or path.startswith("services/hub/"):
                parser.error(f"evidence must be an existing replacement-side file: {path}")
        records[args.source] = {"status": "retired" if args.retire else "ported", "sha256": live[args.source],
                                "replacement": args.replacement, "tests": args.test}
        if args.retire:
            records[args.source]["reason"] = args.reason.strip()
        write(manifest)
    if args.command == "refresh":
        for name, digest in live.items():
            row = records.setdefault(name, {"status": "pending", "replacement": [], "tests": []})
            if row.get("sha256") != digest:
                row["status"] = "pending"
                row["sha256"] = digest
        write(manifest)
    problems = []
    defaults = LEGACY / "cmd/brain/config_defaults.json"
    rust_defaults = ROOT / "services/hub-rs/assets/config-defaults.json"
    if defaults.exists() and json.loads(defaults.read_text()) != json.loads(rust_defaults.read_text()):
        problems.append("Rust config defaults differ from the legacy reference")
    for name, digest in live.items():
        if name not in records or records[name].get("sha256") != digest:
            problems.append(f"Unreviewed legacy source change: {name}")
    complete = 0
    retired = 0
    for name, row in records.items():
        if row["status"] in ("ported", "retired"):
            if row["status"] == "retired":
                retired += 1
                if not row.get("reason", "").strip():
                    problems.append(f"Missing architectural retirement reason: {name}")
            for key in ("replacement", "tests"):
                if not row.get(key):
                    problems.append(f"Missing {key} evidence: {name}")
                for path in row.get(key, []):
                    if not (ROOT / path).is_file():
                        problems.append(f"Missing evidence file: {path}")
                    if path.startswith("services/hub/"):
                        problems.append(f"Evidence still depends on retiring implementation: {path}")
            complete += 1
        elif row["status"] != "pending":
            problems.append(f"Unknown migration status: {name}")
        if name not in live and row["status"] not in ("ported", "retired"):
            problems.append(f"Deleted before migration was recorded: {name}")
    print(f"Legacy source/test files: {len(records)}; ported: {complete - retired}; architecturally retired: {retired}; pending: {len(records) - complete}")
    pending_cutover = [name for name, row in manifest["cutover"].items() if row["status"] != "verified"]
    print("Pending cutover gates: " + ", ".join(pending_cutover))
    if args.command == "ready":
        if complete != len(records) or pending_cutover:
            problems.append("Migration is incomplete; the Go backend cannot be removed.")
        for name, row in manifest["cutover"].items():
            if row["status"] == "verified" and not row["evidence"]:
                problems.append(f"Missing cutover evidence: {name}")
    for problem in problems:
        print(problem)
    return int(bool(problems))


if __name__ == "__main__":
    raise SystemExit(main())
