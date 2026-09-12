#!/usr/bin/env python3
"""Read-only Fly inventory with an allowlisted, credential-free projection."""
import argparse
import json
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("apps", nargs="+")
args = parser.parse_args()
for app in args.apps:
    rows = json.loads(subprocess.check_output(["flyctl", "machine", "list", "-a", app, "--json"]))
    for row in rows:
        config = row.get("config") or {}
        env = config.get("env") or {}
        print(json.dumps({
            "app": app, "id": row["id"], "state": row["state"], "region": row["region"],
            "envNames": sorted(env), "checks": config.get("checks"),
            "image": config.get("image"), "init": config.get("init"),
            "mounts": config.get("mounts"), "services": config.get("services"),
            "guest": config.get("guest"), "restart": config.get("restart"),
            "environment": {k: env[k] for k in (
                "WKS_MACHINE_POWER", "WKS_MACHINE_WAKE", "WKS_MACHINE_IDLE_TIMEOUT", "WKS_MACHINE_IDLE_MODE",
                "WKS_HUB_BIND", "WKS_HUB_PORT", "WKS_NODE_ID", "WKS_HOME", "WKS_DATA",
            ) if k in env},
        }, indent=2))
