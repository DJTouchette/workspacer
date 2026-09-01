//go:build !windows

package routing

import "strings"

func routingPathsEqual(a, b string) bool { return a == b }

func routingPathHasPrefix(path, prefix string) bool { return strings.HasPrefix(path, prefix) }
