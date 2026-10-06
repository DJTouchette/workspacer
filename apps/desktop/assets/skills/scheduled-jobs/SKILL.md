---
name: scheduled-jobs
description: Create or change a Workspacer scheduled job (a recurring or one-off routine the hub runs unattended) by proposing it for the user to approve. Use whenever the user asks for something to happen on a schedule, later, or on demand without them.
---

# Write a scheduled job

Jobs are how Workspacer does work while nobody is watching: on an interval, at
a daily time, once, or on demand, the hub spawns an agent with a prompt, runs a
shell command, or calls a capability. The apps only show jobs and let the user
approve, pause, run or remove them. There is no form, so writing and changing
a job is your part.

Call the `help` tool with topic `jobs` before writing a spec. It is the spec
authority (trigger and action shapes, context steps, guards, limits); write
from it, not from memory.

## New job

1. Pin down what should happen and when, in a question or two. Get exact
   commands, paths and times from the user instead of guessing; a daily `at`
   is the hub's local time.
2. Check the pieces before you commit them to a schedule: run a shell command
   or context step once yourself and look at its output, confirm a `cwd`
   exists, and make sure a `skipUnlessMatch` pattern matches what that output
   really looks like.
3. Prefer a guarded spawn over an unguarded one. When a cheap check can tell
   there is nothing to do, make it a context step with `skipIfEmpty` or
   `skipUnlessMatch` so a quiet run spends no model.
4. Call `propose_job` with the spec.

## Change an existing job

1. `list_jobs` and find the job; `job_history` if the change is about how it
   has been running.
2. Copy its spec, change only what the user asked for, and call `propose_job`
   with the whole new spec plus `"replaces": "<its id>"`.
3. Never remove and re-propose a job to change it; that drops its history and
   leaves the user nothing to compare. Removing or running an existing job is
   fine when the user asks for exactly that.

## Tell the user what happened

A proposal is saved switched off and does nothing until the user approves it
in their Jobs view (Settings → Jobs on the desktop, Jobs in the native app).
Say that plainly, give the job's name, its trigger in words, and what the
action does. Never say a job is scheduled, armed or changed before it has been
approved, and do not try to approve it yourself.
