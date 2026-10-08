#!/usr/bin/env python3
"""Prove the /m-next token drift check bites.

Runs scripts/gen-mobile-tokens.py --check against the checked-in tokens with
(1) the real appearance.rs: must pass; (2) a copy with one native colour
changed: must fail; (3) a copy with a palette field removed: must fail loudly
rather than generate a theme with a hole in it.
"""
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GEN = os.path.join(ROOT, "scripts/gen-mobile-tokens.py")
SOURCE = os.path.join(ROOT, "apps/native/src/appearance.rs")


def check(source):
    return subprocess.run(
        [sys.executable, GEN, "--check", "--source", source],
        capture_output=True,
        text=True,
    )


def with_source(text):
    fd, path = tempfile.mkstemp(suffix=".rs")
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(text)
    return path


def main():
    with open(SOURCE, encoding="utf-8") as f:
        original = f.read()

    ok = check(SOURCE)
    assert ok.returncode == 0, f"unmodified palette must pass:\n{ok.stderr}"

    needle = "base: 0x101113,"
    assert needle in original, "the Dark base colour moved; update this test's needle"
    changed = with_source(original.replace(needle, "base: 0x101114,", 1))
    try:
        drift = check(changed)
        assert drift.returncode == 1, "a changed native colour must fail the check"
        assert "tokens.css" in drift.stderr, drift.stderr
    finally:
        os.unlink(changed)

    syntax_path = os.path.join(ROOT, "apps/native/src/ui/syntax.rs")
    with open(syntax_path, encoding="utf-8") as f:
        syntax = f.read()
    assert 'keyword: "#ff7b72",' in syntax, "GitHub dark keyword colour moved; update this test"
    recoloured = with_source(syntax.replace('keyword: "#ff7b72",', 'keyword: "#ff7b73",', 1))
    try:
        drift = subprocess.run(
            [sys.executable, GEN, "--check", "--syntax", recoloured], capture_output=True, text=True
        )
        assert drift.returncode == 1, "a changed native syntax colour must fail the check"
    finally:
        os.unlink(recoloured)

    holed = with_source(original.replace("                busy: 0xc084fc,\n", "", 1))
    try:
        broken = check(holed)
        assert broken.returncode != 0, "a palette missing a field must not generate"
        assert "missing" in broken.stderr + broken.stdout, broken.stderr
    finally:
        os.unlink(holed)
    print("gen-mobile-tokens: drift check passes on the real palette and fails on drift")


if __name__ == "__main__":
    main()
