#!/usr/bin/env python3
"""Require successful CI for an explicitly requested migration-preview nightly."""
import argparse
import json
import re
import subprocess


def validate_context(event, ref, branch, sha):
    if event != "workflow_dispatch" or ref != f"refs/heads/{branch}":
        raise ValueError("Migration-preview nightlies require manual dispatch from the default branch")
    if not re.fullmatch(r"[0-9a-f]{40}", sha):
        raise ValueError("Expected the exact candidate commit SHA")


def successful_ci(document, branch, sha):
    runs = [run for run in document.get("workflow_runs", [])
            if run.get("head_sha") == sha
            and run.get("head_branch") == branch
            and run.get("event") == "push"
            and run.get("path", "").split("@")[0] == ".github/workflows/ci.yml"]
    if not runs:
        raise ValueError("No push CI run exists for the exact nightly candidate")
    latest = max(runs, key=lambda run: (run.get("run_number", 0), run.get("run_attempt", 0), run.get("id", 0)))
    if latest.get("status") != "completed" or latest.get("conclusion") != "success":
        raise ValueError(f"Candidate CI is not green: {latest.get('status')}/{latest.get('conclusion')}")
    return latest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("repo", "sha", "event", "ref", "default-branch"):
        parser.add_argument(f"--{name}", required=True)
    args = parser.parse_args()
    try:
        validate_context(args.event, args.ref, args.default_branch, args.sha)
        response = subprocess.run([
            "gh", "api", "--method", "GET",
            f"repos/{args.repo}/actions/workflows/ci.yml/runs",
            "-f", f"head_sha={args.sha}", "-f", f"branch={args.default_branch}",
            "-f", "event=push", "-f", "per_page=100",
        ], check=True, capture_output=True, text=True, timeout=30)
        run = successful_ci(json.loads(response.stdout), args.default_branch, args.sha)
    except (ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"Migration-preview nightly refused: {error}\n")
    print(f"Migration-preview candidate CI verified: {run['html_url']}")


if __name__ == "__main__":
    main()
