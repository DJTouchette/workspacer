---
title: Embedded-engine unit fixtures must share the process singleton lock
date: 2026-09-28
suggested_doc: hub-process-supervision
related_paths:
  - Acquire ENGINE_TEST_LOCK for any new actual embedded-engine unit test
  -  or put it in its own integration-test process. Do not weaken the production one-engine guard to make tests pass.
promoted: false
---

# Embedded-engine unit fixtures must share the process singleton lock

## Observation
Claudemons EmbeddedDaemon intentionally refuses a second active runtime in one process. Adding the live capability-inventory unit fixture exposed parallel lib-test contention with captured-manager-message and remote-proxy engine fixtures:171 lib tests passed serially, while one failed concurrently at startup with engine already active. A shared cfg(test) backend::ENGINE_TEST_LOCK now serializes those three actual-engine unit fixtures. The other in-memory Hub tests remain parallel and integration binaries have independent process singletons.
