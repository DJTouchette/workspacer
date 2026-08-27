#!/usr/bin/env bash
#
# fetch-release.sh — install a workspacer box image from a RELEASE ARTIFACT
# instead of compiling the tree.
#
# Runs INSIDE a `docker build`, in the `binaries-artifact` / `hubfiles-artifact`
# stage of deploy/fly/node/Dockerfile and deploy/fly/hub/Dockerfile. It
# downloads `workspacer-server-<platform>.<ext>` from a GitHub release,
# verifies that what came back IS the release that was asked for, and lays the
# files out under a destination directory in the SAME shape the source-build
# stages produce — so the runtime stage's COPY lines do not know or care which
# mode built them.
#
# ---------------------------------------------------------------------------
# WHY THE STAMP CHECK IS THE POINT OF THIS FILE
# ---------------------------------------------------------------------------
# A tag on a GitHub release is MUTABLE. `nightly` in particular is rolled every
# night: the tag is deleted and recreated against a new commit. So "I asked for
# nightly and the download succeeded" says nothing at all about what is now
# inside the image, and neither does anything else on the box — as of this
# writing `workspacer`, `hub` and `brain` have no `--version` flag at all, and
# `claudemon --version` prints a Cargo version (`0.1.0`) that has not moved in
# the life of the project. There is no honest way to ask a running box what code
# it is.
#
# `build-stamp` is that answer, written by the release workflow at package time
# (.github/workflows/release.yml, "Bundle standalone server"). This script fails
# the build LOUDLY when the stamp disagrees with what was requested, rather than
# producing an image that runs the wrong commit and says nothing. verify-image.sh
# then asserts the stamp is present in the finished image, and both entrypoints
# print it on every boot.
#
# An archive with NO stamp is also a hard failure: it is a release built before
# the stamp existed, which is exactly the case where you cannot tell what you
# got. Build that tag from source instead.
#
# ---------------------------------------------------------------------------
# CONFIGURATION (environment, because a Dockerfile passes build ARGs this way)
# ---------------------------------------------------------------------------
#   WKS_RELEASE_TAG       required. `nightly`, or a `v*` tag. Checked against the
#                         stamp's own `tag=`.
#   WKS_RELEASE_SHA       optional. The commit the release is expected to carry.
#                         Compared against the stamp's `commit=` on the common
#                         prefix (>= 7 chars), so a short sha is accepted.
#   WKS_RELEASE_REPO      default DJTouchette/workspacer
#   WKS_RELEASE_ASSET     default workspacer-server-linux-x64.tar.gz
#   WKS_RELEASE_BASE_URL  default https://github.com/<repo>/releases/download
#                         Overridable so the test suite can point at file:// and
#                         prove this logic without a network or a release.
#   WKS_RELEASE_TOKEN_FILE  optional. A file holding a GitHub token, sent as a
#                         bearer header. Meant for a BuildKit `--mount=type=
#                         secret`, NOT a build ARG: an ARG is recorded in the
#                         image history.
#   WKS_RELEASE_REQUIRE   space-separated paths that must exist inside the
#                         archive. Default is the node's set. The hub passes its
#                         own, shorter, list.
#   WKS_RELEASE_CHMOD     space-separated paths to make executable. Default is
#                         the four binaries plus mcp.
#
# Usage: fetch-release.sh <dest-dir>
#
set -euo pipefail

DEST="${1:-}"
[ -n "$DEST" ] || { echo "fetch-release.sh: usage: fetch-release.sh <dest-dir>" >&2; exit 2; }

TAG="${WKS_RELEASE_TAG:-}"
SHA="${WKS_RELEASE_SHA:-}"
REPO="${WKS_RELEASE_REPO:-DJTouchette/workspacer}"
ASSET="${WKS_RELEASE_ASSET:-workspacer-server-linux-x64.tar.gz}"
BASE_URL="${WKS_RELEASE_BASE_URL:-https://github.com/${REPO}/releases/download}"
TOKEN_FILE="${WKS_RELEASE_TOKEN_FILE:-}"
REQUIRE="${WKS_RELEASE_REQUIRE:-workspacer hub brain claudemon mcp}"
CHMOD="${WKS_RELEASE_CHMOD:-workspacer hub brain claudemon mcp}"

die() {
  printf '\n' >&2
  printf 'fetch-release.sh: FAILED — %s\n' "$1" >&2
  shift
  for line in "$@"; do printf '  %s\n' "$line" >&2; done
  printf '\n' >&2
  exit 1
}

say() { printf 'fetch-release.sh: %s\n' "$*"; }

[ -n "$TAG" ] || die "WKS_RELEASE_TAG is empty." \
  "Artifact mode has to name a release. Pass --build-arg WKS_RELEASE_TAG=nightly" \
  "(or a v* tag), or build from source with --build-arg WKS_INSTALL=source."

# ---------------------------------------------------------------------------
# 1. Download.
# ---------------------------------------------------------------------------
url="${BASE_URL}/${TAG}/${ASSET}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
archive="$work/$ASSET"

say "downloading $url"
curl_args=(--fail --location --silent --show-error --retry 3 --retry-delay 2 --output "$archive")
if [ -n "$TOKEN_FILE" ] && [ -s "$TOKEN_FILE" ]; then
  say "using the bearer token from $TOKEN_FILE"
  curl_args+=(--header "Authorization: Bearer $(tr -d '\r\n' <"$TOKEN_FILE")")
fi
curl "${curl_args[@]}" "$url" || die "could not download $url" \
  "Check the tag exists and carries that asset:" \
  "  gh release view $TAG --repo $REPO" \
  "A private repo needs a token — see WKS_RELEASE_TOKEN_FILE in this file's header."

[ -s "$archive" ] || die "$url returned an empty file."

# Not a verification — there is no published checksum to compare against — but
# it is the one value that identifies this exact download, so it belongs in the
# build log next to the stamp.
if command -v sha256sum >/dev/null 2>&1; then
  say "sha256(archive) = $(sha256sum "$archive" | cut -d' ' -f1)"
fi
say "size(archive)   = $(wc -c <"$archive" | tr -d ' ') bytes"

# ---------------------------------------------------------------------------
# 2. Unpack.
# ---------------------------------------------------------------------------
unpacked="$work/unpacked"
mkdir -p "$unpacked"
case "$ASSET" in
  *.tar.gz|*.tgz) tar -C "$unpacked" -xzf "$archive" ;;
  *.zip)          unzip -q "$archive" -d "$unpacked" ;;
  *)              die "don't know how to unpack $ASSET (expected .tar.gz or .zip)." ;;
esac

# The bundle is one directory (`workspacer-server/`), not a pile of loose files.
# Resolve it rather than hardcoding the name: the release step owns that name and
# a rename there should not silently produce an empty image here.
roots=()
while IFS= read -r entry; do roots+=("$entry"); done < <(find "$unpacked" -mindepth 1 -maxdepth 1)
if [ "${#roots[@]}" -eq 1 ] && [ -d "${roots[0]}" ]; then
  root="${roots[0]}"
else
  # A flat archive is legal too; treat the extraction dir itself as the root.
  root="$unpacked"
fi
say "archive root    = $(basename "$root")"

# ---------------------------------------------------------------------------
# 3. THE DRIFT GUARD. Everything above this point only proves a download.
# ---------------------------------------------------------------------------
stamp="$root/build-stamp"
[ -f "$stamp" ] || die "the archive carries no build-stamp." \
  "That means this release predates the stamp, so there is no way to tell which" \
  "commit is inside it — which is the entire reason artifact mode refuses to" \
  "guess. Either rebuild the release from a tree that has the stamping step in" \
  ".github/workflows/release.yml, or build this image from source:" \
  "  --build-arg WKS_INSTALL=source"

stamp_get() {
  # First match wins; values may contain '=' (an ISO timestamp does not, but a
  # future key might), so cut from the first '=' only.
  grep -E "^$1=" "$stamp" 2>/dev/null | head -n1 | cut -d= -f2-
}

stamp_tag="$(stamp_get tag)"
stamp_commit="$(stamp_get commit)"
stamp_version="$(stamp_get version)"

[ -n "$stamp_tag" ] || die "the build-stamp has no tag= line." \
  "The stamp is malformed; treat this artifact as unidentified." \
  "stamp contents:" "$(cat "$stamp")"

if [ "$stamp_tag" != "$TAG" ]; then
  die "RELEASE DRIFT: asked for tag '$TAG', the archive says it is '$stamp_tag'." \
    "The asset behind that tag is not the release you named. \`nightly\` is rolled" \
    "nightly and a mirror or a CDN can serve a stale one; a v* tag can be moved." \
    "Nothing about the image would have shown this at runtime, which is why the" \
    "build stops here." \
    "stamp contents:" "$(cat "$stamp")"
fi

if [ -n "$SHA" ]; then
  [ -n "$stamp_commit" ] || die "WKS_RELEASE_SHA=$SHA was given, but the stamp has no commit= line." \
    "stamp contents:" "$(cat "$stamp")"
  n="${#SHA}"
  [ "${#stamp_commit}" -lt "$n" ] && n="${#stamp_commit}"
  [ "$n" -ge 7 ] || die "the expected sha and the stamp's commit share fewer than 7 characters to compare." \
    "expected=$SHA stamp=$stamp_commit"
  if [ "${SHA:0:$n}" != "${stamp_commit:0:$n}" ]; then
    die "COMMIT DRIFT: expected commit $SHA, the archive was built from $stamp_commit." \
      "Map a sha back to the release it shipped in with:" \
      "  gh release view $TAG --repo $REPO --json targetCommitish,tagName" \
      "stamp contents:" "$(cat "$stamp")"
  fi
  say "commit verified: $stamp_commit matches the expected $SHA"
fi

say "stamp verified: tag=$stamp_tag version=${stamp_version:-<none>} commit=${stamp_commit:-<none>}"

# ---------------------------------------------------------------------------
# 4. Completeness. A bundle missing `mcp` builds an image whose entrypoint dies
#    ten seconds into every boot; catching it here costs nothing.
# ---------------------------------------------------------------------------
missing=()
for path in $REQUIRE; do
  [ -e "$root/$path" ] || missing+=("$path")
done
if [ "${#missing[@]}" -gt 0 ]; then
  die "the archive is missing $((${#missing[@]})) file(s) this image needs: ${missing[*]}" \
    "The release bundle and the box images have to agree on their contents — see" \
    ".github/workflows/release.yml, step 'Bundle standalone server'." \
    "If you are building an OLD tag whose bundle predates a file, build from source:" \
    "  --build-arg WKS_INSTALL=source"
fi

# ---------------------------------------------------------------------------
# 5. Install.
# ---------------------------------------------------------------------------
for path in $CHMOD; do
  [ -e "$root/$path" ] && chmod 0755 "$root/$path"
done

mkdir -p "$DEST"
cp -a "$root/." "$DEST/"
say "installed into $DEST:"
find "$DEST" -maxdepth 1 -mindepth 1 -printf '  %f\n' | sort

# The stamp travels with the files. The runtime stage copies it to
# /usr/local/share/workspacer/build-stamp, verify-image.sh asserts it, and the
# entrypoint prints it on every boot.
[ -f "$DEST/build-stamp" ] || die "internal: the stamp did not survive the copy into $DEST."
say "OK"
