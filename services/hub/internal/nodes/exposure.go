package nodes

// WHO CAN READ nodes.json — and the platform on which that question has an
// answer at all.
//
// nodes.json holds a Fly API token that can start, stop and create machines,
// i.e. spend money. The hub therefore wants to say, at startup, "this file is
// readable by more than you". This file is about the fact that the OBVIOUS way
// to say it is wrong on one of the three platforms this project ships on.
//
// The obvious way was `os.Stat(path).Mode().Perm()&0o077 != 0`, and it shipped
// as FileLooksExposed. On Unix that is exactly right. On Windows it is not
// merely imprecise, it is CONSTANT: os.fileStat.Mode (os/types_windows.go)
// synthesises the permission bits from a single attribute —
//
//	if FILE_ATTRIBUTE_READONLY { m |= 0444 } else { m |= 0666 }
//
// — so Perm() is 0666 for every writable file and 0444 for every read-only
// one, and `&0o077` is non-zero in BOTH. The check answered "exposed" for
// every file that exists, having read nothing about who can actually open it.
// os.Chmod on Windows is the same story from the other side: it only toggles
// FILE_ATTRIBUTE_READONLY (syscall/syscall_windows.go), so `chmod 0644` grants
// nobody anything.
//
// That is a worse failure than an absent check. A warning printed on every
// Windows start regardless of the file's real ACL is a warning the user learns
// to scroll past, and the one time it means something it looks the same as the
// thousand times it did not. CI caught it as a red
// TestFileLooksExposedNoticesLoosePermissions on the containment-windows job —
// the test was right and the check was wrong.
//
// So the answer is not to skip the test on Windows (this repo's CI
// deliberately makes a host that cannot run a guard RED rather than
// green-with-a-skip — see internal/sweepguard.Gate). It is to make the check
// mean something on Windows, where permissions are ACLs, and to admit
// three-valued ignorance where it genuinely cannot decide:
//
//   - [ExposureLoose]     — established: a principal beyond the owner can read it.
//   - [ExposureOwnerOnly] — established: only the owner (and SYSTEM /
//     Administrators, who can read anything on the machine anyway) can.
//   - [ExposureUnknown]   — NOT established. Say so, do not imply either.
//
// The third value is the point. A boolean forces "could not tell" to be spelt
// either as a false alarm or as a clean bill of health, and both are lies.
//
// The Unix and Windows halves live in exposure_unix.go and exposure_windows.go.

// Exposure is what the hub could establish about who can read a file. The
// zero value is [ExposureUnknown] on purpose: a caller that forgets to switch
// on this gets "we do not know", never "it is fine".
type Exposure int

const (
	// ExposureUnknown means the hub could not determine who can read the file.
	// It is NOT a synonym for safe.
	ExposureUnknown Exposure = iota
	// ExposureOwnerOnly means only the owner — plus the machine's own
	// administrative principals, which are not a leak — can read it.
	ExposureOwnerOnly
	// ExposureLoose means a principal beyond the owner can read it, and the
	// file holds a credential that spends money.
	ExposureLoose
)

func (e Exposure) String() string {
	switch e {
	case ExposureOwnerOnly:
		return "owner-only"
	case ExposureLoose:
		return "loose"
	default:
		return "unknown"
	}
}

// FileExposure reports who can read path, plus a one-line explanation in the
// caller's own words — WHY it decided that, or what it could not read. The
// explanation is empty only for [ExposureOwnerOnly]; both other answers owe
// the user a reason, because both other answers ask them to go look.
//
// A path that cannot be examined is [ExposureUnknown], never "fine".
func FileExposure(path string) (Exposure, string) {
	return fileExposure(path)
}
