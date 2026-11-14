#!/usr/bin/env python3
"""Point the retired Electron app's nightly update feed at the final Electron release.

v0.170.0 was the last Electron release. An installed Electron NIGHTLY polls
https://github.com/<repo>/releases/download/nightly/latest*.yml (electron-
updater's generic provider) and resolves each file's `url`/`path` against that
folder, where no Electron installer exists any more. This rewrites those two
keys in copies of v0.170.0's latest*.yml to absolute download URLs of the
assets on that release, so a last Electron nightly updates itself to the final
Electron build. Everything else, the sha512 values included, is left byte for
byte: the files they describe are unchanged.

electron-updater 6.8.9 resolves the value with `new URL(value, baseUrl)`
(out/providers/Provider.js resolveFiles, out/util.js newUrlFromBase), so an
absolute URL replaces the base. That holds for the generic provider only: the
GitHub provider (stable installs) prefixes its own download path, which is why
stable releases carry these files UNCHANGED instead.

A file naming an installer the release does not hold is skipped with a
warning instead of publishing a feed that would 404.
"""
import argparse
import json
from pathlib import Path
import re
import sys
from urllib.parse import quote

KEY = re.compile(r'^(?P<lead>\s*(?:-\s+)?)(?P<key>url|path):[ \t]+(?P<value>.*?)[ \t]*(?P<eol>\r?\n?)$')


def asset_for(value, assets):
    """The release asset a feed entry names, allowing for GitHub's renaming.

    GitHub stores a space in an uploaded name as a dot, electron-builder's
    metadata writes it as a dash, and a hand-corrected feed has the dot.
    """
    name = value.rsplit('/', 1)[-1]
    candidates = [name, name.replace(' ', '-'), name.replace(' ', '.')]
    for candidate in candidates:
        if candidate in assets:
            return candidate
    squash = lambda text: re.sub(r'[ .\-]', '', text)
    matches = [asset for asset in assets if squash(asset) == squash(name)]
    return matches[0] if len(matches) == 1 else None


def unquote(value):
    if len(value) >= 2 and value[0] == value[-1] and value[0] in '\'"':
        return value[1:-1]
    return value


def rewrite(text, repo, tag, assets):
    """Return the rewritten feed, or raise ValueError naming what is missing."""
    base = f'https://github.com/{repo}/releases/download/{tag}/'
    out, rewritten = [], 0
    for line in text.splitlines(keepends=True):
        match = KEY.match(line)
        if not match:
            out.append(line)
            continue
        value = unquote(match['value'])
        if not value:
            raise ValueError(f"empty {match['key']} entry")
        if re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*://', value):
            if not value.startswith(base):
                raise ValueError(f'entry already points elsewhere: {value}')
            value = value[len(base):]
        asset = asset_for(value, assets)
        if asset is None:
            raise ValueError(f'{tag} has no asset for {value}')
        out.append(f"{match['lead']}{match['key']}: {base}{quote(asset)}{match['eol']}")
        rewritten += 1
    if rewritten == 0:
        raise ValueError('no url or path entry found')
    return ''.join(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--repo', required=True, help='owner/name')
    parser.add_argument('--tag', required=True, help='the final Electron release, e.g. v0.170.0')
    parser.add_argument('--assets', required=True,
                        help='JSON file: `gh release view <tag> --json assets` output')
    parser.add_argument('--out', required=True, help='directory for the rewritten feeds')
    parser.add_argument('feeds', nargs='*')
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', args.repo):
        sys.exit('invalid repository name')
    if not re.fullmatch(r'v\d+\.\d+\.\d+', args.tag):
        sys.exit('invalid release tag')
    listing = json.loads(Path(args.assets).read_text(encoding='utf8'))
    assets = {asset['name'] for asset in listing.get('assets', [])}
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    written = 0
    for feed in map(Path, args.feeds):
        if not re.fullmatch(r'latest[A-Za-z0-9-]*\.yml', feed.name):
            sys.exit(f'not an update feed file: {feed.name}')
        try:
            # newline='' keeps the file's own line endings.
            with open(feed, encoding='utf8', newline='') as handle:
                text = rewrite(handle.read(), args.repo, args.tag, assets)
        except ValueError as error:
            print(f'::warning::Electron nightly feed {feed.name} not published: {error}')
            continue
        with open(out / feed.name, 'w', encoding='utf8', newline='') as handle:
            handle.write(text)
        written += 1
        print(f'{feed.name}: points at {args.tag}')
    print(f'{written} Electron nightly feed file(s) written to {out}')


if __name__ == '__main__':
    main()
