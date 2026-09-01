package routing

import "testing"

func TestWindowsRoutingPathComparisonIsCaseInsensitiveButVolumeBounded(t *testing.T) {
	if !windowsRoutingPathsEqual(`C:\Work\Client`, `c:\work\client`) {
		t.Error("case variants of one Windows path did not compare equal")
	}
	if !windowsRoutingPathHasPrefix(`c:\WORK\Client\sub`, `C:\work\client\`) {
		t.Error("case variants of a Windows ancestor did not compare as contained")
	}
	if windowsRoutingPathHasPrefix(`C:\work\client-old`, `C:\work\client\`) {
		t.Error("a prefix sibling compared as contained")
	}
	if windowsRoutingPathHasPrefix(`D:\work\client\sub`, `C:\work\client\`) {
		t.Error("a path on another volume compared as contained")
	}
}
