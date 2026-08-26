// Package statelost tells a genuine FIRST RUN apart from state that has
// VANISHED.
//
// Several files here are create-once-then-keep-forever: the pairing credential
// <config>/workspacer/remote-token, every setting in
// <config>/workspacer/config.yaml, the Web Push application-server keypair in
// <config>/workspacer-hub/vapid.json. Each was loaded by code shaped
//
//	read the file; if it is not there, make a new one
//
// which is exactly right the first time and exactly wrong every time after,
// because a recreated credential is a DIFFERENT credential and recreated
// settings are DIFFERENT settings. The process comes up printing a healthy
// banner while rejecting everything that was paired against the old value, or
// running a configuration nobody chose. Nothing logs anything, because from the
// loader's point of view nothing went wrong.
//
// This package answers the one question those loaders could not ask: is the
// directory around the missing file empty — nobody has ever run here — or does
// it still hold the rest of the state, in which case something took this one
// file away.
//
// The check is deliberately coarse, and the asymmetry is the point. A false
// positive is a loud message with a documented way past it. A false negative is
// the silent failure this package exists to end.
//
// # WHY AN EMPTY SUBDIRECTORY IS NOT EVIDENCE
//
// One correction to that coarseness, and it was earned rather than reasoned:
// counting ANY entry meant counting a directory somebody's installer had just
// made. On the Fly node, deploy/fly/node/bootstrap.sh pre-creates plugins/,
// library/, layouts/, sessions/ and logs/ inside <config>/workspacer before the
// brain ever starts, so the brain's config.yaml check reported STATE LOSS on
// every genuinely-first boot, in the boot log, with a paragraph about restoring
// from a backup that did not exist. Observed on a real image, not inferred.
//
// A guard that is wrong on every first boot is a guard the operator learns to
// scroll past, which costs the two cases it exists for. So an entry only counts
// as evidence when it holds something: a file of any size, or a directory that
// is not empty. A bare mkdir proves a mkdir ran. It does not prove the program
// ran.
//
// deploy/fly/hub/bootstrap.sh reached the same correction independently, in
// shell, for the same reason, see its bs_snapshot_dir. This is the Go half of
// one rule, not a second rule.
//
// What that gives up is narrow: state that has been reduced to empty
// directories and nothing else now reads as a first run. That shape means the
// files were deleted while the directories were left, which is what a partial
// wipe looks like, and on the node it is exactly what a fresh bootstrap looks
// like too. The two are indistinguishable from here, and the caller that cannot
// afford to guess should not be guessing: deploy/fly/hub/bootstrap.sh records a
// per-file marker under state/seen/ and consults that FACT first, using this
// inference only as a second opinion.
package statelost

import (
	"errors"
	"io"
	"os"
	"path/filepath"
)

// Suspected reports whether `name` is missing from `dir` in a way that looks
// like loss rather than a first run: the directory exists and still holds at
// least one entry, other than `name` itself, that carries actual state.
//
// An entry carries state if it is anything but an empty directory. See the
// package comment for why a bare mkdir is not evidence.
//
// A directory that cannot be read at all (including one that does not exist) is
// reported as NOT suspected — nobody has ever run there, so there is nothing to
// have lost. Callers that need to distinguish an unreadable directory from an
// absent one should stat it themselves; every caller here treats both as "carry
// on and create", which is what they did before this package existed.
func Suspected(dir, name string) bool {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return false
	}
	for _, e := range entries {
		if e.Name() == name {
			continue
		}
		if e.IsDir() && isEmptyDir(filepath.Join(dir, e.Name())) {
			continue
		}
		return true
	}
	return false
}

// isEmptyDir reports whether path is a directory that holds nothing.
//
// A directory that cannot be read is deliberately NOT called empty. Unreadable
// is unknown, and the safe answer for a guard biased toward the loud outcome is
// to let the entry count as evidence.
func isEmptyDir(path string) bool {
	f, err := os.Open(path)
	if err != nil {
		return false
	}
	defer f.Close()
	// One name is all it takes to disprove emptiness; io.EOF is the empty case
	// and every other error is an unreadable directory.
	if _, err := f.Readdirnames(1); err != nil {
		return errors.Is(err, io.EOF)
	}
	return false
}
