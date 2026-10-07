#!/usr/bin/env python3
"""Exercise the portable launch-asset generator against isolated source bytes."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class LaunchAssetsTest(unittest.TestCase):
    def test_utf8_crlf_hash_and_stale_detection_are_locale_independent(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            script = root / "scripts/generate-rust-launch-assets.py"
            script.parent.mkdir()
            shutil.copyfile(Path(__file__).with_name(script.name), script)
            desktop = root / "apps/desktop"
            skills = desktop / "assets/skills"
            skills.mkdir(parents=True)
            (skills / "plugins.json").write_bytes(json.dumps({
                "workspacer": {"description": "Ordinary \u00e9", "skills": {
                    "spawn-agent": "before spawning", "project-brief": "for briefs"}},
                "workspacer-fleet": {"description": "Fleet", "skills": {"standup": "for status"}},
            }).encode("utf-8"))
            bodies = {}
            for name in ("project-brief", "spawn-agent", "standup"):
                body = "# " + name + "\r\nUnicode: \u00e9 \U0001f680\r\n"
                bodies[name] = body
                path = skills / name / "SKILL.md"
                path.parent.mkdir(parents=True)
                path.write_bytes(body.encode("utf-8"))
            (skills / "spawn-agent/references").mkdir()
            (skills / "spawn-agent/references/more.md").write_bytes(b"more\n")
            shared = desktop / "src/main/shared"
            shared.mkdir(parents=True)
            (shared / "managerDoctrine.ts").write_bytes(b"const MANAGER_PREAMBLE = `manager`;\r\n")
            (shared / "fleetWorkflow.ts").write_bytes(b"export const WORKFLOW_DISCOVERY = 'workflow';\r\n")
            target = root / "services/hub-rs/assets/launch-instructions.json"
            target.parent.mkdir(parents=True)
            env = dict(os.environ, LC_ALL="C", PYTHONUTF8="0", PYTHONCOERCECLOCALE="0")
            def run(*args):
                return subprocess.run([sys.executable, str(script), *args], env=env, capture_output=True)
            generated = run()
            self.assertEqual(generated.returncode, 0, generated.stderr)
            raw = target.read_bytes()
            self.assertNotIn(b"\r\n", raw)
            data = json.loads(raw)
            files = data["files"]
            self.assertEqual(list(files), sorted(files))
            self.assertEqual(files["workspacer/skills/spawn-agent/SKILL.md"], bodies["spawn-agent"])
            self.assertEqual(files["workspacer/skills/spawn-agent/references/more.md"], "more\n")
            self.assertEqual(files["workspacer-fleet/skills/standup/SKILL.md"], bodies["standup"])
            self.assertNotIn("workspacer/skills/standup/SKILL.md", files)
            self.assertEqual(json.loads(files["workspacer/.claude-plugin/plugin.json"]),
                             {"name": "workspacer", "description": "Ordinary \u00e9"})
            self.assertEqual(data["plugins"], {"workspacer": ["spawn-agent", "project-brief"],
                                               "workspacer-fleet": ["standup"]})
            self.assertIn("{skill:spawn-agent}", data["instructions"]["workspacer"]["pointer"])
            self.assertIn("{dir}", data["instructions"]["workspacer"]["native"])
            packed = json.dumps(files, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            self.assertEqual(data["version"], hashlib.sha256(packed).hexdigest()[:16])
            self.assertEqual(run("--check").returncode, 0)
            target.write_bytes(raw.replace(b"manager", b"outdated", 1))
            self.assertNotEqual(run("--check").returncode, 0)


if __name__ == "__main__":
    unittest.main()
