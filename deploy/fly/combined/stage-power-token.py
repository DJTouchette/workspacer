#!/usr/bin/env python3
"""Create an app-scoped power credential and stage it without printing it."""
import argparse
import base64
import json
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("app")
args = parser.parse_args()
result = subprocess.run(["flyctl", "tokens", "create", "deploy", "-a", args.app,
    "--name", "workspacer-self-power", "--expiry", "8760h", "--json"],
    check=True, capture_output=True, text=True)
data = json.loads(result.stdout)
def values(obj):
    if isinstance(obj, dict):
        for value in obj.values(): yield from values(value)
    elif isinstance(obj, list):
        for value in obj: yield from values(value)
    elif isinstance(obj, str): yield obj
tokens = [value for value in values(data) if value.startswith("FlyV1 ")]
if len(tokens) != 1:
    raise SystemExit("Token response was not recognized; no credential was printed or staged")
encoded = base64.b64encode(tokens[0].encode()).decode()
subprocess.run(["flyctl", "secrets", "import", "--stage", "-a", args.app],
    input="WORKSPACER_POWER_TOKEN_B64=" + encoded + "\n", text=True, check=True)
print("App-scoped power credential staged; worker environments will not receive it")
