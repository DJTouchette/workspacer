#!/usr/bin/env bash
# Compatibility entrypoint for the reviewed Rust image contract.
set -euo pipefail
role=${WKS_RUST_IMAGE_ROLE:-node}
if [ -x /usr/local/lib/wks-rust/verify-image.sh ]; then
  exec /usr/local/lib/wks-rust/verify-image.sh "$role"
fi
exec "$(dirname "$0")/../rust/verify-image.sh" "$role"
