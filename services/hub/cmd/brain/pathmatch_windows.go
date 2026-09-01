//go:build windows

package main

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

// Windows opens drive letters, UNC hosts/shares, and ordinary NTFS components
// case-insensitively. CompareStringOrdinal is the Windows identifier primitive:
// unlike strings.EqualFold, it does not apply generic Unicode equivalences that
// NTFS does not use for path identity.
func canonicalPathsEqual(a, b string) bool {
	return compareWindowsPathUTF16(windowsPathUTF16(a), windowsPathUTF16(b))
}

func canonicalPathHasPrefix(path, prefix string) bool {
	path16 := windowsPathUTF16(path)
	prefix16 := windowsPathUTF16(prefix)
	if len(path16) < len(prefix16) {
		return false
	}
	return compareWindowsPathUTF16(path16[:len(prefix16)], prefix16)
}

func windowsPathUTF16(path string) []uint16 {
	// CanonicalizePath has already normalized every accepted path. Encoding
	// explicitly gives CompareStringOrdinal counted strings, so an embedded NUL
	// cannot shorten an identifier at the syscall boundary.
	return utf16.Encode([]rune(path))
}

func compareWindowsPathUTF16(a, b []uint16) bool {
	if len(a) == 0 || len(b) == 0 {
		return len(a) == len(b)
	}
	if len(a) > maxCompareStringOrdinalLen || len(b) > maxCompareStringOrdinalLen {
		// A Windows path cannot approach this bound. A native-comparison failure
		// must never turn into an accidental non-match or a wider fallback root.
		panic("brain: Windows path is too long for CompareStringOrdinal")
	}

	result, _, callErr := procCompareStringOrdinal.Call(
		uintptr(unsafe.Pointer(&a[0])), uintptr(len(a)),
		uintptr(unsafe.Pointer(&b[0])), uintptr(len(b)),
		1, // TRUE is the only accepted numeric value for bIgnoreCase.
	)
	// The pointers cross a uintptr syscall boundary, so keep their backing
	// arrays live until the native call has returned.
	runtime.KeepAlive(a)
	runtime.KeepAlive(b)
	if result == 0 {
		panic("brain: CompareStringOrdinal failed: " + callErr.Error())
	}
	return result == compareStringOrdinalEqual
}
