#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

FILES=(
  "services/claudemon/README.md"
  "services/hub-rs/MIGRATION.md"
  "apps/desktop/README.md"
  "apps/tui/README.md"
)

WORD_EDGE='(^|[^[:alnum:]_])'
PATTERN="still stubs|${WORD_EDGE}stubs?([^[:alnum:]_]|$)|${WORD_EDGE}planned([^[:alnum:]_]|$)|next milestones?|not implemented"

for file in "${FILES[@]}"; do
  if [[ ! -f "$ROOT/$file" || ! -r "$ROOT/$file" ]]; then
    echo "Required component document missing or unreadable: $file" >&2
    exit 2
  fi
done

# grep status1 means no matches; status2+ means the scan was not completed.
status=0
matches="$(cd "$ROOT" && grep -EnHi "$PATTERN" "${FILES[@]}")" || status=$?
if (( status > 1 )); then
  echo "Component documentation scan failed (grep status $status)." >&2
  exit "$status"
fi

if [[ -z "$matches" ]]; then
  echo "No stale maturity phrases found in component documents."
  exit 0
fi

cat <<'EOF'
Potential stale maturity language found in component documents.
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
