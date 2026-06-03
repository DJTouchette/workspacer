//go:build !race

package bus

// raceEnabled is false in normal (non -race) builds. See raceflag_race_test.go.
const raceEnabled = false
