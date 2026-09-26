#!/usr/bin/env python3
"""Check local paths and inline links in curated Rivet docs (stdlib only).

Run from any directory: python3 scripts/check-rivet-docs.py
Pairs with `rivet context lint`, which validates metadata and retrieval quality.
Checks related_paths globs, explicit repo-root source paths, and inline Markdown
links (including Markdown heading and HTML id fragments). Historical learnings
are deliberately excluded. This is not a semantic or external-URL validator.
"""

import argparse
import glob
from html.parser import HTMLParser
from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit


def prose(text):
    """Remove fenced examples so their sample paths/links aren't treated as docs."""
    return re.sub(r"(?ms)^\s*(`{3,}|~{3,})[^\n]*\n.*?^\s*\1\s*$", "", text)


class IDs(HTMLParser):
    def __init__(self):
        super().__init__()
        self.ids = set()

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if key == "id" or (tag == "a" and key == "name"):
                self.ids.add(value)


def fragments(path):
    text = path.read_text(encoding="utf-8")
    parser = IDs()
    parser.feed(text)
    found = parser.ids
    if path.suffix.lower() in {".md", ".markdown"}:
        seen = {}
        for heading in re.findall(r"(?m)^#{1,6}\s+(.+?)\s*#*\s*$", prose(text)):
            slug = re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-")
            count = seen.get(slug, 0)
            seen[slug] = count + 1
            found.add(slug if not count else f"{slug}-{count}")
    return found


def check(root):
    documents = sorted((root / ".rivet/context").rglob("*.md"))
    documents += sorted((root / ".rivet/runbooks").rglob("*.md"))
    errors = []
    references = 0
    if not documents:
        return ["No curated Rivet documents found"], 0, 0
    for doc in documents:
        text = doc.read_text(encoding="utf-8")
        body = prose(text)

        def error(message):
            errors.append(f"{doc.relative_to(root)}: {message}")

        # The current corpus uses block sequences for related_paths. Rivet's
        # own lint owns general YAML parsing; don't pretend this is a YAML parser.
        front = re.match(r"\A---\n(.*?)\n---(?:\n|$)", text, re.S)
        if front:
            paths = re.search(r"(?m)^related_paths:\s*\n((?:[ \t]+[^\n]*\n?)*)", front[1])
            if paths:
                for raw in re.findall(r"(?m)^\s+-\s+(.+)$", paths[1]):
                    path = raw.strip().strip("\"'")
                    references += 1
                    if not glob.glob(str(root / path), recursive=True):
                        error(f"related_paths matches nothing: {path}")

        # Only explicit repository-root paths: bare filenames, URLs, runtime
        # paths, relative examples and pseudo-code are outside this check.
        for code in re.findall(r"`([^`\n]+)`", body):
            if not re.match(r"^(apps|services|contracts|landing|docs|scripts)/", code):
                continue
            path = re.split(r"#|::|:\d", code, maxsplit=1)[0]
            if re.search(r"\s|[{}<>]|\.\.\.", path):
                continue
            # Extensionless names can be unbuilt binaries; dotted Go package
            # symbols are not files. Require a recognizable source suffix or
            # an existing directory before treating the code span as a path.
            if not (root / path).is_dir() and not re.search(
                r"\.(?:md|go|rs|tsx?|jsx?|mjs|cjs|json|ya?ml|toml|html|css|sh|py|sql)(?:\*|/)?$", path
            ):
                continue
            references += 1
            if not glob.glob(str(root / path), recursive=True):
                error(f"missing repository path: {code}")

        for target in re.findall(r"\[[^\]\n]*\]\(([^)\n]+)\)", body):
            # Inline destinations optionally use <...> or a quoted title.
            target = target.strip()
            if target.startswith("<") and ">" in target:
                target = target[1:target.index(">")]
            else:
                target = target.split(' "', 1)[0]
            url = urlsplit(target)
            if url.scheme or url.netloc:
                continue
            path = (doc.parent / unquote(url.path)).resolve() if url.path else doc
            references += 1
            if not path.exists():
                error(f"broken local link: {target}")
            elif url.fragment and path.is_file():
                if path.suffix.lower() in {".md", ".markdown", ".html", ".htm"}:
                    if unquote(url.fragment) not in fragments(path):
                        error(f"missing anchor: {target}")
    return errors, len(documents), references


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    errors, count, references = check(args.root.resolve())
    for error in errors:
        print(error, file=sys.stderr)
    print(f"Checked {count} curated Rivet docs, {references} local references: {len(errors)} errors")
    return bool(errors)


if __name__ == "__main__":
    sys.exit(main())
