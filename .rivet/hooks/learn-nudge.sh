#!/bin/bash
# Rivet learn nudge — fires after recon MCP tool calls.
# If Claude has used multiple recon tools but hasn't called rivet.learn,
# nudge it to record findings.

input=$(cat)

# Extract transcript_path.
transcript_path=""
if command -v jq >/dev/null 2>&1; then
  transcript_path=$(echo "$input" | jq -r '.transcript_path // empty' 2>/dev/null)
elif command -v python3 >/dev/null 2>&1; then
  transcript_path=$(echo "$input" | python3 -c "import sys,json; print(json.load(sys.stdin).get('transcript_path',''))" 2>/dev/null)
fi

if [ -z "$transcript_path" ] || [ ! -f "$transcript_path" ]; then
  exit 0
fi

# Count recon tool calls in this session.
recon_count=$(grep -cE "recon\.(grep|search|related|context|symbols)" "$transcript_path" 2>/dev/null || echo 0)

# Only nudge after 2+ recon calls (indicates real investigation, not a quick lookup).
if [ "$recon_count" -lt 2 ]; then
  exit 0
fi

# Check if rivet.learn was already called.
if grep -q "rivet\.learn" "$transcript_path" 2>/dev/null; then
  exit 0
fi

# Output nudge as additionalContext so Claude sees it.
echo "If you've discovered anything non-obvious during this investigation (hidden dependencies, performance traps, implicit ordering, gotchas), call rivet.learn with a title and observation. The entry lands in .rivet/learnings/ and is later promoted into a context doc."
