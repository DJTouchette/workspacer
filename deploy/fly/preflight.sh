#!/usr/bin/env bash
# Default deployment checks use the Rust images. No cloud operations.
set -euo pipefail
exec "$(dirname "$0")/rust/preflight.sh" "$@"
