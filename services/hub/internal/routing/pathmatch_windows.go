//go:build windows

package routing

import (
	"runtime"
	"unicode/utf16"
	"unsafe"

	"golang.org/x/sys/windows"
)

const (
	compareStringOrdinalEqual  = 2
	maxCompareStringOrdinalLen = 1<<31 - 1
)

var procCompareStringOrdinal = windows.NewLazySystemDLL("kernel32.dll").NewProc("CompareStringOrdinal")

// Windows resolves drive letters, UNC hosts/shares, and ordinary NTFS path
// components case-insensitively. A ceiling must answer for the directory the
// filesystem opens, not for the caller's or routing.yaml's casing of it.
// CompareStringOrdinal is the Windows primitive for non-linguistic identifiers
// such as NTFS filenames. In ignore-case mode it uses the operating system's
// uppercase table without applying generic Unicode equivalences (notably the
// Kelvin sign's fold to K).
func routingPathsEqual(a, b string) bool {
	return compareWindowsRoutingPathUTF16(windowsRoutingPathUTF16(a), windowsRoutingPathUTF16(b))
}

func routingPathHasPrefix(path, prefix string) bool {
	path16 := windowsRoutingPathUTF16(path)
	prefix16 := windowsRoutingPathUTF16(prefix)
	if len(path16) < len(prefix16) {
		return false
	}
	return compareWindowsRoutingPathUTF16(path16[:len(prefix16)], prefix16)
}

func windowsRoutingPathUTF16(path string) []uint16 {
	// filepath values on Windows originate as Unicode. Encoding explicitly lets
	// us pass counted strings to CompareStringOrdinal, so an embedded NUL cannot
	// silently turn a comparison into a shorter null-terminated one.
	return utf16.Encode([]rune(path))
}

func compareWindowsRoutingPathUTF16(a, b []uint16) bool {
	if len(a) == 0 || len(b) == 0 {
		return len(a) == len(b)
	}
	if len(a) > maxCompareStringOrdinalLen || len(b) > maxCompareStringOrdinalLen {
		// A Windows path cannot approach this bound. Treat reaching it as an
		// invariant failure instead of returning "not equal", which could make a
		// configured ancestor miss and fall through to a weaker default ceiling.
		panic("routing: Windows path is too long for CompareStringOrdinal")
	}

	result, _, callErr := procCompareStringOrdinal.Call(
		uintptr(unsafe.Pointer(&a[0])), uintptr(len(a)),
		uintptr(unsafe.Pointer(&b[0])), uintptr(len(b)),
		1, // TRUE is the only accepted numeric value for bIgnoreCase.
	)
	// The pointers cross a uintptr syscall boundary, so keep both backing arrays
	// live until the Windows call has returned.
	runtime.KeepAlive(a)
	runtime.KeepAlive(b)
	if result == 0 {
		// All arguments above are valid by construction. If the native primitive
		// nevertheless fails, stopping is the fail-closed outcome: returning false
		// here would make a real configured ceiling look like no match.
		panic("routing: CompareStringOrdinal failed: " + callErr.Error())
	}
	return result == compareStringOrdinalEqual
}
