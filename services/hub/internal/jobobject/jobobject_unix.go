//go:build !windows

package jobobject

// Confine is a no-op off Windows — process-group semantics and parentwatch
// already cover Unix.
func Confine() error { return nil }
