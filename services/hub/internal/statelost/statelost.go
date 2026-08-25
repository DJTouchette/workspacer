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
package statelost

import "os"

// Suspected reports whether `name` is missing from `dir` in a way that looks
// like loss rather than a first run: the directory exists and still holds at
// least one entry that is not `name` itself.
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
		if e.Name() != name {
			return true
		}
	}
	return false
}
