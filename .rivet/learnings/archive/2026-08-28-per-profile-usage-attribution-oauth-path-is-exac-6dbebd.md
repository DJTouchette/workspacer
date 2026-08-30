---
title: Per-profile usage attribution: OAuth path is exact, but transcript/DB paths are guesses and profile tokens expire when idle
date: 2026-08-28
confidence: high
suggested_doc: usage-accounting
related_paths:
  - services/claudemon/src/session/account_usage.rs
  - apps/desktop/src/renderer/src/lib/claudeAccount.ts
  - apps/desktop/src/main/services/sessionStore/analyticsWriter.ts
promoted: true
promoted_to: usage-accounting
---

# Per-profile usage attribution: OAuth path is exact, but transcript/DB paths are guesses and profile tokens expire when idle

## Observation
Three attribution facts verified on a live machine (2026-08-28). (1) EXACT path: `fetch_account_usage(client, root)` already takes a config root and reads `<root>/.credentials.json`; the profile dir ~/.claude/accounts/work/.credentials.json exists with its OWN subscriptionType ("team", rateLimitTier default_claude_max_5x) distinct from the default root's ("max", default_claude_max_20x). So per-profile account windows are genuinely fetchable, not inferred. (2) BUT the Work profile's OAuth token was EXPIRED (expiresAt 2026-08-20, 8 days stale) — `token_from_credentials` bails locally on expiry and never refreshes (rotation is the CLI's job), so a boot-time fetch for any profile you have not used recently silently returns nothing. Idle profiles are exactly the ones a boot readout is for. (3) GUESS territory: `~/.claude/accounts/work/projects` is a SYMLINK to the shared ~/.claude/projects, so both logins' transcripts land in one physical directory. `claudeAccountOf()` (renderer/src/lib/claudeAccount.ts) and `root_from_transcript()` only work because the CLI's path STRING retains the profile root — a canonicalized path loses the account entirely. And workspacer.db's session_history/session_model_usage have no profile/account/transcript column at all (columns verified: session_id cwd agent_name model git_branch started_at ended_at duration_ms input_tokens output_tokens cost_usd peak_context tool_calls message_count subagent_count workflow_runs workflow_failed status updated_at provider), so the 746 existing history rows cannot be retroactively attributed to a profile by any means.

## Impact
A per-profile usage feature that folds transcripts or reads workspacer.db will produce plausible but wrong numbers. Only the OAuth-per-root path is trustworthy, and it degrades to "unknown" for idle profiles.

## Recommendation
Never canonicalize/realpath a Claude transcript path before deriving the account. If per-profile history is wanted, add an account column going forward and leave old rows unattributed rather than backfilling a guess.
