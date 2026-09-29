// go-duration-reference captures Go's duration parser for cross-language tests.
// Usage: go run tools/go-duration-reference/main.go '10m' '2562048h'
// It uses only the standard library and never contacts a power provider.
package main

import (
	"encoding/json"
	"os"
	"strings"
	"time"
)

func main() {
	type result struct {
		Input        string `json:"input"`
		Milliseconds *int64 `json:"milliseconds"`
	}
	rows := make([]result, 0, len(os.Args)-1)
	for _, input := range os.Args[1:] {
		row := result{Input: input}
		if duration, err := time.ParseDuration(strings.TrimSpace(input)); err == nil {
			ms := duration.Milliseconds()
			row.Milliseconds = &ms
		}
		rows = append(rows, row)
	}
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}
