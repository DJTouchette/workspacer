#!/usr/bin/env python3
"""Generate/check Rust launch assets from authoritative desktop source assets.

The agent skill plugins mirror apps/desktop/scripts/gen-agent-skill-plugins.mjs
byte for byte (same layout, sort order, hash and instruction templates); the
desktop check-agent-skill-plugins.mjs asserts the two outputs are equal.
"""
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
# Match Node's readFileSync(..., 'utf8') exactly, including CRLF. Locale-default
# text I/O can change both Unicode content and the immutable version on Windows.
def utf8(path):
    return path.read_bytes().decode("utf-8")

skills_dir = desktop / "assets/skills"
plugins = json.loads(utf8(skills_dir / "plugins.json"))
files = {}
members = {}
instructions = {}
for plugin, spec in plugins.items():
    skills = list(spec["skills"])
    members[plugin] = skills
    manifest = {"name": plugin, "description": spec["description"]}
    files[f"{plugin}/.claude-plugin/plugin.json"] = json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
    for skill in skills:
        base = skills_dir / skill
        for path in base.rglob("*"):
            if path.is_file():
                files[f"{plugin}/skills/{skill}/{path.relative_to(base).as_posix()}"] = utf8(path)
    uses = list(spec["skills"].items())
    instructions[plugin] = {
        "native": "Workspacer loads these skills into this session: "
        + ", ".join(f"{skill} ({use})" for skill, use in uses)
        + ". If one is missing from your available skills, read its SKILL.md under {dir}.",
        "pointer": "Workspacer provides these skills: "
        + ", ".join(
            f"{'and ' if i == len(uses) - 1 else ''}read {{skill:{skill}}} {use}"
            for i, (skill, use) in enumerate(uses)
        )
        + ".",
    }
files = {key: files[key] for key in sorted(files)}
version = hashlib.sha256(json.dumps(files, ensure_ascii=False, separators=(",", ":")).encode()).hexdigest()[:16]
doctrine = utf8(desktop / "src/main/shared/managerDoctrine.ts")
match = re.search(r"const MANAGER_PREAMBLE = `([^`]+)`;", doctrine)
if not match or "${" in match[1]:
    raise SystemExit("Manager doctrine is no longer a static template; update the generator")
workflow = utf8(desktop / "src/main/shared/fleetWorkflow.ts")
policy = re.search(r"export const WORKFLOW_DISCOVERY =\s*'([^'\\]*)';", workflow)
if not policy:
    raise SystemExit("Workflow discovery is no longer a static string; update the generator")
assets = {
    "version": version,
    "plugins": members,
    "instructions": instructions,
    "files": files,
    "manager": match[1] + "\n\nSELECTED FLEET POLICY: " + policy[1],
}
target = root / "services/hub-rs/assets/launch-instructions.json"
text = json.dumps(assets, ensure_ascii=False, indent=2) + "\n"
if args.check:
    if not target.exists() or target.read_bytes() != text.encode("utf-8"):
        raise SystemExit("Rust launch assets are stale; run scripts/generate-rust-launch-assets.py")
else:
    target.write_bytes(text.encode("utf-8"))
