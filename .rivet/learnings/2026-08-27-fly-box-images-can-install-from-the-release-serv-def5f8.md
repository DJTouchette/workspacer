---
title: Fly box images can install from the release server bundle; the build stamp is the only honest version
date: 2026-08-27
confidence: high
suggested_doc: workspacer-serve-cli
promoted: false
---

# Fly box images can install from the release server bundle; the build stamp is the only honest version

## Observation
deploy/fly/{node,hub}/Dockerfile now take `--build-arg WKS_INSTALL=artifact` (default stays `source`), which downloads `workspacer-server-<platform>.tar.gz` from a GitHub release instead of compiling. BuildKit skips the gobuild/rustbuild/webbuild stages entirely because the artifact arm does not reference them — measured 11s vs ~10min, asserted from a --no-cache build log in preflight's new ARTIFACT stage.

The release bundle was missing `mcp` (the node entrypoint dies 10s into boot without it) and any version identity; both were added to .github/workflows/release.yml's "Bundle standalone server" step. The identity gap is real and was previously undocumented: `workspacer`, `hub` and `brain` have NO --version flag, and `claudemon --version` prints the Cargo version 0.1.0 which has not moved in the life of the project. So nothing on a deployed box could say which commit it ran.

`build-stamp` (key=value, written only by deploy/fly/write-build-stamp.sh) closes that. It is installed at /usr/local/share/workspacer/build-stamp (+ build-stamp.hub on the hub layer), asserted by verify-image.sh, and printed by both entrypoints on every boot so `fly logs` answers it without a shell. `install=source|release` records provenance; fetch-release.sh copies a bundle's stamp VERBATIM rather than rewriting it.</observation>
<parameter name="impact">Three drift guards fail the build loudly: RELEASE DRIFT (bundle downloads fine but claims another tag — the real `nightly`-is-rolled case), COMMIT DRIFT (right tag, wrong sha), STAMP DRIFT (artifact hub on an artifact base from a different commit, caught by verify-image.sh comparing the two stamps). Today's published `nightly` predates the stamp and lacks `mcp`, so artifact mode correctly REFUSES it — the first nightly built after this change is what unblocks the real download path.</impact>
<parameter name="recommendation">Never add a fourth writer of the stamp format; write-build-stamp.sh is the single writer for CI and both Dockerfiles. Anything added to the box images that must come out of the release bundle also has to be added to the release packaging step and to the Dockerfile's WKS_RELEASE_REQUIRE list, or artifact mode ships an image that cannot boot. Offline coverage is deploy/fly/test-fetch-release.sh (51 assertions, curl over file://); `./deploy/fly/preflight.sh artifact` is the docker-level proof.</recommendation>
<parameter name="related_paths">["deploy/fly/fetch-release.sh", "deploy/fly/write-build-stamp.sh", "deploy/fly/test-fetch-release.sh", "deploy/fly/node/Dockerfile", "deploy/fly/hub/Dockerfile", "deploy/fly/node/verify-image.sh", "deploy/fly/preflight.sh", ".github/workflows/release.yml"]
