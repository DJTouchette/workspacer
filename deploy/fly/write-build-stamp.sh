#!/usr/bin/env bash
#
# write-build-stamp.sh — write the one file that says what a workspacer build IS.
#
# THE FORMAT IS A CONTRACT WITH FOUR READERS, so it is written in exactly one
# place, here:
#
#   .github/workflows/release.yml   stamps the release bundle at package time
#   deploy/fly/fetch-release.sh     refuses an artifact whose stamp disagrees
#                                   with the tag/sha that was asked for
#   deploy/fly/node/verify-image.sh asserts the stamp exists in the built image
#   deploy/fly/{node,hub}/entrypoint.sh  prints it on every boot
#
# The format is deliberately the dumbest thing that works: one `key=value` per
# line, lowercase keys, no quoting, no comments, no blank lines. It is read by
# `grep | cut` in a Debian build stage that has no jq, and by `tr '\n' ' '` in an
# entrypoint that wants it on one log line. JSON would need a parser in four
# places to buy nothing.
#
# WHY IT EXISTS AT ALL: nothing else on a box can answer "what code is this?".
# `workspacer`, `hub` and `brain` have no --version flag; `claudemon --version`
# prints the Cargo version `0.1.0`, which has not moved in the life of the
# project. A rolled `nightly` tag makes the tag itself no answer either. This
# file is the first honest one.
#
# Usage: write-build-stamp.sh <dest-file>
#
# Every value comes from the environment so the three writers can each supply
# what they actually know. An unset value is written as `unknown` rather than
# omitted: a reader should never have to distinguish "key missing" from "value
# unknown", and a fixed key set keeps the one-line boot log the same shape.
#
#   WKS_STAMP_COMPONENT  what these files are: server | hub | ...   (default server)
#   WKS_STAMP_INSTALL    how they got here:    source | release     (default source)
#                        `source`  = compiled by the docker build that is
#                                    installing them, from this working tree.
#                        `release` = built by .github/workflows/release.yml and
#                                    downloaded as a published release asset.
#                        The stamp travels VERBATIM with the files it describes:
#                        fetch-release.sh installs the bundle's own stamp rather
#                        than rewriting it, so provenance is never overwritten by
#                        whoever happened to copy it last.
#   WKS_STAMP_VERSION    the product version, e.g. 0.150.0-nightly.202608270800
#   WKS_STAMP_TAG        the release tag these files were published under
#   WKS_STAMP_COMMIT     the git sha they were built from
#   WKS_STAMP_BUILT      RFC3339 UTC build time  (default: now)
#   WKS_STAMP_PLATFORM   e.g. linux-x64
#   WKS_STAMP_RUN        CI run id / build reference, if there is one
#
set -euo pipefail

DEST="${1:-}"
[ -n "$DEST" ] || { echo "write-build-stamp.sh: usage: write-build-stamp.sh <dest-file>" >&2; exit 2; }

v() { printf '%s' "${1:-unknown}" | tr -d '\r\n'; }

mkdir -p "$(dirname "$DEST")"
{
  printf 'component=%s\n' "$(v "${WKS_STAMP_COMPONENT:-server}")"
  printf 'install=%s\n'   "$(v "${WKS_STAMP_INSTALL:-source}")"
  printf 'version=%s\n'   "$(v "${WKS_STAMP_VERSION:-}")"
  printf 'tag=%s\n'       "$(v "${WKS_STAMP_TAG:-}")"
  printf 'commit=%s\n'    "$(v "${WKS_STAMP_COMMIT:-}")"
  printf 'built=%s\n'     "$(v "${WKS_STAMP_BUILT:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}")"
  printf 'platform=%s\n'  "$(v "${WKS_STAMP_PLATFORM:-}")"
  printf 'run=%s\n'       "$(v "${WKS_STAMP_RUN:-}")"
} >"$DEST"

cat "$DEST"
