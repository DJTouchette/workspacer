// Package jobobject confines the calling process — and every descendant it
// spawns — inside a Windows Job Object configured to KILL_ON_JOB_CLOSE.
//
// The hub supervises a tree of children (the brain, plugin sidecars, install
// steps). On Unix these die with the hub via signals and parentwatch; on
// Windows there is no process-group SIGTERM, TerminateProcess doesn't touch
// grandchildren, and a force-killed hub would orphan the whole tree with its
// ports still bound. Assigning ourselves to a kill-on-close job makes the OS
// the guarantor: when the hub exits — cleanly, crashed, or Task-Manager-killed
// — the job's last handle closes and Windows terminates every process in it.
//
// On non-Windows platforms Confine is a no-op.
package jobobject
