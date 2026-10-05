---
title: Fake stream sessions only record assistant text from content_block_delta
date: 2026-10-05
confidence: medium
suggested_doc: claudemon-providers
related_paths:
  - services/hub-rs/tests/fixtures/fake_claude_title_session.py
  - apps/native/tests/auto_titles_hub.rs
promoted: false
---

# Fake stream sessions only record assistant text from content_block_delta

## Observation
In a fake claude stream-json fixture, a full {type:assistant,message:{content:[text]}} frame did not produce an assistant_text item in /sessions/:id/conversation; only stream_event content_block_delta text_delta did (the transcript tailer is the other feed and a fake writes no transcript). Also after Backend shutdown/restart the embedded daemon had no rows for fake sessions (no provider transcript to restore), so restart checks must read the hub journal (data/agent-launches.json). A hub refuses a pre-seeded config dir without remote-token (STATE LOSS guard): start once, stop, then write config.
