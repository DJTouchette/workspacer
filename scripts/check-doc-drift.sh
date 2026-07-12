#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

FILES=(
  "services/claudemon/README.md"
  "services/hub/README.md"
  "apps/desktop/README.md"
  "apps/tui/README.md"
)

WORD_EDGE='(^|[^[:alnum:]_])'
PATTERN="still stubs|${WORD_EDGE}stubs?([^[:alnum:]_]|$)|${WORD_EDGE}planned([^[:alnum:]_]|$)|next milestones?|not implemented"

matches="$(
  cd "$ROOT"
  grep -EnHi "$PATTERN" "${FILES[@]}" || true
)"

if [[ -z "$matches" ]]; then
  echo "No stale maturity phrases found in component READMEs."
  exit 0
fi

cat <<'EOF'
Potential stale maturity language found in component READMEs.
Review these lines before release; docs/features.md should remain the detailed
source of truth for maturity claims.

EOF
printf '%s\n' "$matches"

if [[ "${WKS_DOC_DRIFT_STRICT:-0}" == "1" ]]; then
  exit 1
fi

cat <<'EOF'

Informational only. Set WKS_DOC_DRIFT_STRICT=1 to make this check fail.
EOF
