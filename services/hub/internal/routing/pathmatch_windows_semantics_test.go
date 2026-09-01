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
	if windowsRoutingPathsEqual(`C:\Work\Kelvin`, `c:\work\kelvin`) {
		t.Error("Windows routing path comparison used Unicode folding; only ASCII A-Z may fold")
	}
}

func TestWindowsRoutingPathComparisonHandlesUNCBoundaries(t *testing.T) {
	if !windowsRoutingPathsEqual(`\\Server\Share\Work`, `\\server\share\work`) {
		t.Error("case variants of one UNC path did not compare equal")
	}
	if !windowsRoutingPathHasPrefix(`\\SERVER\SHARE\Work\child`, `\\server\share\work\`) {
		t.Error("a child on the same UNC share did not compare as contained")
	}
	if windowsRoutingPathHasPrefix(`\\server\share-old\work`, `\\server\share\`) {
		t.Error("a sibling UNC share compared as contained")
	}
	if windowsRoutingPathHasPrefix(`\\server\other\work`, `\\server\share\`) {
		t.Error("a path on another UNC share compared as contained")
	}
}
