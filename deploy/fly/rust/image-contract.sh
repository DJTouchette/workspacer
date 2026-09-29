#!/usr/bin/env sh
# Preserve the base-image downstream verification entrypoint.
exec /usr/local/lib/wks-rust/verify-image.sh "${WKS_RUST_IMAGE_ROLE:?image role missing}"
