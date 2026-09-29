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
            files = {}
            for name in ("project-brief", "spawn-agent"):
                body = "# " + name + "\r\nUnicode: \u00e9 \U0001f680\r\n"
                files[name + "/SKILL.md"] = body
                path = desktop / ("assets/skills/" + name + "/SKILL.md")
                path.parent.mkdir(parents=True)
                path.write_bytes(body.encode("utf-8"))
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
            self.assertEqual(data["files"], files)
            packed = json.dumps(files, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            self.assertEqual(data["version"], hashlib.sha256(packed).hexdigest()[:16])
            self.assertEqual(run("--check").returncode, 0)
            target.write_bytes(raw.replace(b"manager", b"outdated", 1))
            self.assertNotEqual(run("--check").returncode, 0)


if __name__ == "__main__":
    unittest.main()
