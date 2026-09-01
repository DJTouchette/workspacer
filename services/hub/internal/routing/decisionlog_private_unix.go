//go:build !windows

package routing

import "os"

func openDecisionLogFile(path string) (*os.File, error) {
	return os.OpenFile(path, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600)
}

func secureDecisionLogFile(f *os.File) error { return f.Chmod(0o600) }
