#!/usr/bin/env python3
"""Generate/check Rust launch assets from authoritative desktop source assets."""
import argparse
import hashlib
import json
import pathlib
import re

root = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--check", action="store_true")
args = parser.parse_args()
desktop = root / "apps/desktop"
files = {f"{name}/SKILL.md": (desktop / f"assets/skills/{name}/SKILL.md").read_text() for name in ("project-brief", "spawn-agent")}
version = hashlib.sha256(json.dumps(files, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()[:16]
doctrine = (desktop / "src/main/shared/managerDoctrine.ts").read_text()
match = re.search(r"const MANAGER_PREAMBLE = `([^`]+)`;", doctrine)
if not match or "${" in match[1]:
    raise SystemExit("Manager doctrine is no longer a static template; update the generator")
workflow = (desktop / "src/main/shared/fleetWorkflow.ts").read_text()
policy = re.search(r"export const WORKFLOW_DISCOVERY =\s*'([^'\\]*)';", workflow)
if not policy:
    raise SystemExit("Workflow discovery is no longer a static string; update the generator")
assets = {"version": version, "files": files, "manager": match[1] + "\n\nSELECTED FLEET POLICY: " + policy[1]}
target = root / "services/hub-rs/assets/launch-instructions.json"
text = json.dumps(assets, ensure_ascii=False, indent=2) + "\n"
if args.check:
    if not target.exists() or target.read_text() != text:
        raise SystemExit("Rust launch assets are stale; run scripts/generate-rust-launch-assets.py")
else:
    target.write_text(text)
