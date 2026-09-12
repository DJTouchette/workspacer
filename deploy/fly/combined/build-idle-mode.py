#!/usr/bin/env python3
"""Build a policy-only image from an already power-enabled isolated deployment."""
import argparse
import json
import pathlib
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("base", help="Existing power-enabled image (prefer its digest)")
parser.add_argument("image", help="New image tag")
parser.add_argument("app")
parser.add_argument("machine")
parser.add_argument("mode", choices=("observe", "stop", "off"))
args = parser.parse_args()
power = json.loads(subprocess.check_output([
    "docker", "run", "--rm", "--entrypoint", "cat", args.base,
    "/opt/combined/power.json",
]))
if power.get("app") != args.app or power.get("machine") != args.machine:
    raise SystemExit("Power target identity mismatch; refusing to build")
if power.get("mode") not in ("observe", "stop", "off"):
    raise SystemExit("Unrecognized base power policy")
power["mode"] = args.mode
with tempfile.TemporaryDirectory(prefix="workspacer-idle-mode-") as directory:
    stage = pathlib.Path(directory)
    (stage / "power.json").write_text(json.dumps(power) + "\n")
    (stage / "Dockerfile").write_text(
        "ARG BASE\nFROM ${BASE}\n"
        "COPY --chown=0:0 --chmod=0644 power.json /opt/combined/power.json\n"
    )
    subprocess.run([
        "docker", "build", "--build-arg", "BASE=" + args.base,
        "-t", args.image, directory,
    ], check=True)
print(json.dumps(power, indent=2))
