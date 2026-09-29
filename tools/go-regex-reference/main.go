// Optional standard-library oracle for the Rust persisted-job regexp adapter.
// It is not used by runtime builds, CI tests, or packaged applications.
package main

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"sort"
	"strconv"
	"unicode"
)

type Meta struct {
	GoVersion      string `json:"goVersion"`
	UnicodeVersion string `json:"unicodeVersion"`
}
type Case struct {
	Pattern string `json:"pattern"`
	Text    string `json:"text"`
	Invalid bool   `json:"invalid,omitempty"`
	Matches *bool  `json:"matches,omitempty"`
}
type Cases struct {
	Meta  Meta   `json:"meta"`
	Cases []Case `json:"cases"`
}

func emit(v any) {
	e := json.NewEncoder(os.Stdout)
	e.SetIndent("", "  ")
	if err := e.Encode(v); err != nil {
		panic(err)
	}
}
func main() {
	if len(os.Args) < 2 {
		panic("usage: go run tools/go-regex-reference/main.go cases FILE | names")
	}
	meta := Meta{runtime.Version(), unicode.Version}
	switch os.Args[1] {
	case "cases":
		if len(os.Args) != 3 {
			panic("cases requires one JSON fixture path")
		}
		raw, err := os.ReadFile(os.Args[2])
		if err != nil {
			panic(err)
		}
		var doc Cases
		if json.Unmarshal(raw, &doc) != nil {
			if err = json.Unmarshal(raw, &doc.Cases); err != nil {
				panic(err)
			}
		}
		if len(doc.Cases) == 0 {
			panic("empty regexp fixture")
		}
		for i := range doc.Cases {
			c := &doc.Cases[i]
			r, err := regexp.Compile(c.Pattern)
			if c.Invalid {
				if err == nil {
					panic(fmt.Sprintf("expected invalid pattern %q", c.Pattern))
				}
				c.Matches = nil
				continue
			}
			if err != nil {
				panic(err)
			}
			matched := r.MatchString(c.Text)
			c.Matches = &matched
		}
		doc.Meta = meta
		emit(doc)
	case "names":
		candidates := map[string]bool{"Any": true, "Assigned": true, "ASCII": true, "Cn": true, "LC": true}
		for name := range unicode.Categories {
			candidates[name] = true
		}
		for name := range unicode.Scripts {
			candidates[name] = true
		}
		// Read optional alias tables through Go's parser so this tool still builds
		// on toolchains where unicode.CategoryAliases does not exist as an API.
		file, err := parser.ParseFile(token.NewFileSet(), filepath.Join(runtime.GOROOT(), "src", "unicode", "tables.go"), nil, 0)
		if err != nil {
			panic(err)
		}
		ast.Inspect(file, func(node ast.Node) bool {
			decl, ok := node.(*ast.ValueSpec)
			if !ok {
				return true
			}
			for _, name := range decl.Names {
				if name.Name != "CategoryAliases" {
					continue
				}
				for _, value := range decl.Values {
					literal, ok := value.(*ast.CompositeLit)
					if !ok {
						continue
					}
					for _, entry := range literal.Elts {
						pair, ok := entry.(*ast.KeyValueExpr)
						if !ok {
							continue
						}
						key, ok := pair.Key.(*ast.BasicLit)
						if !ok {
							continue
						}
						text, err := strconv.Unquote(key.Value)
						if err == nil {
							candidates[text] = true
						}
					}
				}
			}
			return true
		})
		accepted := []string{}
		for name := range candidates {
			if _, err := regexp.Compile(`\p{` + name + `}`); err == nil {
				accepted = append(accepted, name)
			}
		}
		sort.Strings(accepted)
		probes := map[string]bool{}
		for _, name := range []string{"Cn", "LC", "Cased_Letter", "cased letter", "greek", "White_Space", "Script=Greek"} {
			_, err := regexp.Compile(`\p{` + name + `}`)
			probes[name] = err == nil
		}
		emit(struct {
			Meta     Meta            `json:"meta"`
			Accepted []string        `json:"accepted"`
			Probes   map[string]bool `json:"probes"`
		}{meta, accepted, probes})
	default:
		panic("unknown mode")
	}
}
