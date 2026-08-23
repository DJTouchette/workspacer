#!/bin/bash
# Rivet compact check — fires after rivet MCP tool calls.
# If un-promoted learnings exceed threshold, nudge to promote.

LEARNING_THRESHOLD=10

if [ ! -d ".rivet/learnings" ]; then
  exit 0
fi

# Count *.md files directly under .rivet/learnings/ (exclude archive/) that
# are not already marked as promoted.
total=0
promoted=0
for f in .rivet/learnings/*.md; do
  [ -f "$f" ] || continue
  total=$((total+1))
  if grep -q "^promoted: true" "$f" 2>/dev/null; then
    promoted=$((promoted+1))
  fi
done
active=$((total-promoted))

if [ "$active" -ge "$LEARNING_THRESHOLD" ]; then
  echo "Learning log has ${active} active entries (threshold: ${LEARNING_THRESHOLD}). Run /rivet-promote-learnings to review, merge, and promote high-value entries into context docs."
fi
