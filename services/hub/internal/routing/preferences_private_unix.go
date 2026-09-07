//go:build !windows

package routing

import "os"

func createPreferencesTemp(dir string) (*os.File, error) {
	return os.CreateTemp(dir, ".routing-preferences-*")
}
