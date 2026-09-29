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
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "services/hub-rs/migration.json"
LEGACY = ROOT / "services/hub"


def sources():
    return {
        str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest()
        for path in sorted(LEGACY.rglob("*.go"))
        if "cmd/hub-reference/" not in str(path)
    }


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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["refresh", "status", "check", "ready", "record"])
    parser.add_argument("--source")
    parser.add_argument("--replacement", action="append", default=[])
    parser.add_argument("--test", action="append", default=[])
    parser.add_argument("--retire", action="store_true", help="Record an architectural responsibility removed by the new ownership model")
    parser.add_argument("--reason", help="Required explanation for architectural retirement")
    args = parser.parse_args()
    manifest = read()
    live = sources()
    records = manifest["files"]
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
        MANIFEST.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    if args.command == "refresh":
        for name, digest in live.items():
            row = records.setdefault(name, {"status": "pending", "replacement": [], "tests": []})
            if row.get("sha256") != digest:
                row["status"] = "pending"
                row["sha256"] = digest
        MANIFEST.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
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
