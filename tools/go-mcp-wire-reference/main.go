// Optional source-only capture of the retired Go facade's typed JSON wire rules.
// Standard library only: neither the hub nor the Go MCP SDK is built or started.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"flag"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
)

type Rule struct {
	Kind    string          `json:"kind"`
	Omit    string          `json:"omit,omitempty"`
	Pointer bool            `json:"pointer,omitempty"`
	Fields  map[string]Rule `json:"fields,omitempty"`
	Item    *Rule           `json:"item,omitempty"`
}
type Input struct {
	Type        string `json:"type"`
	Rule        *Rule  `json:"rule,omitempty"`
	Passthrough string `json:"passthrough,omitempty"`
}
type Capture struct {
	SourceSHA256 map[string]string `json:"sourceSha256"`
	Inputs       map[string]Input  `json:"inputs"`
}

func read(path string) []byte {
	b, e := os.ReadFile(path)
	if e != nil {
		panic(e)
	}
	return b
}
func digest(b []byte) string { h := sha256.Sum256(b); return hex.EncodeToString(h[:]) }
func literal(expr ast.Expr) string {
	if lit, ok := expr.(*ast.BasicLit); ok && lit.Kind == token.STRING {
		s, e := strconv.Unquote(lit.Value)
		if e != nil {
			panic(e)
		}
		return s
	}
	return ""
}
func typeName(expr ast.Expr) string {
	switch t := expr.(type) {
	case *ast.Ident:
		return t.Name
	case *ast.SelectorExpr:
		return typeName(t.X) + "." + t.Sel.Name
	case *ast.StarExpr:
		return "*" + typeName(t.X)
	}
	return ""
}
func main() {
	root := flag.String("root", ".", "repository root")
	check := flag.String("check", "", "compare capture with JSON file")
	flag.Parse()
	dir := filepath.Join(*root, "services/hub/cmd/mcp")
	files, e := os.ReadDir(dir)
	if e != nil {
		panic(e)
	}
	var catalog map[string][]struct {
		Name string `json:"name"`
	}
	if e = json.Unmarshal(read(filepath.Join(*root, "services/hub-rs/assets/mcp-tools.json")), &catalog); e != nil {
		panic(e)
	}
	names := map[string]bool{}
	for _, tools := range catalog {
		for _, tool := range tools {
			names[tool.Name] = true
		}
	}
	capture := Capture{SourceSHA256: map[string]string{}, Inputs: map[string]Input{}}
	types := map[string]ast.Expr{}
	var funcs []*ast.FuncDecl
	for _, entry := range files {
		n := entry.Name()
		if entry.IsDir() || !strings.HasSuffix(n, ".go") || strings.HasSuffix(n, "_test.go") {
			continue
		}
		raw := read(filepath.Join(dir, n))
		capture.SourceSHA256["services/hub/cmd/mcp/"+n] = digest(raw)
		file, e := parser.ParseFile(token.NewFileSet(), n, raw, 0)
		if e != nil {
			panic(e)
		}
		for _, decl := range file.Decls {
			switch d := decl.(type) {
			case *ast.GenDecl:
				for _, spec := range d.Specs {
					if t, ok := spec.(*ast.TypeSpec); ok {
						types[t.Name.Name] = t.Type
					}
				}
			case *ast.FuncDecl:
				funcs = append(funcs, d)
			}
		}
	}
	register := func(name, typ string) {
		if !names[name] {
			return
		}
		if previous, ok := capture.Inputs[name]; ok && previous.Type != typ {
			panic("ambiguous input " + name)
		}
		capture.Inputs[name] = Input{Type: typ}
	}
	direct := map[string]string{"addSpawnTool": "spawnAgentIn", "addGateTool": "gateIn", "addConversationTool": "conversationIn", "addConfigSaveTool": "@object", "addObjectTool": "@object", "addRoutingPreferencesGetTool": "listAgentsIn"}
	for _, function := range funcs {
		literals := map[string]bool{}
		ast.Inspect(function.Body, func(n ast.Node) bool {
			if x, ok := n.(ast.Expr); ok {
				if s := literal(x); names[s] {
					literals[s] = true
				}
			}
			return true
		})
		ast.Inspect(function.Body, func(n ast.Node) bool {
			call, ok := n.(*ast.CallExpr)
			if !ok {
				return true
			}
			if generic, ok := call.Fun.(*ast.IndexExpr); ok {
				if fn, ok := generic.X.(*ast.Ident); ok && strings.HasPrefix(fn.Name, "add") && len(call.Args) > 1 {
					register(literal(call.Args[1]), typeName(generic.Index))
				}
			}
			if fn, ok := call.Fun.(*ast.Ident); ok {
				if typ, ok := direct[fn.Name]; ok && len(call.Args) > 1 {
					register(literal(call.Args[1]), typ)
				}
				if fn.Name == "addUiTool" && len(call.Args) > 2 {
					if callback, ok := call.Args[len(call.Args)-1].(*ast.FuncLit); ok {
						register(literal(call.Args[2]), typeName(callback.Type.Params.List[0].Type))
					}
				}
			}
			if fn, ok := call.Fun.(*ast.SelectorExpr); ok && typeName(fn.X) == "mcp" && fn.Sel.Name == "AddTool" && len(call.Args) > 0 {
				if callback, ok := call.Args[len(call.Args)-1].(*ast.FuncLit); ok {
					for _, p := range callback.Type.Params.List {
						for _, name := range p.Names {
							if name.Name == "in" {
								for name := range literals {
									register(name, typeName(p.Type))
								}
							}
						}
					}
				}
			}
			return true
		})
	}
	var shape func(ast.Expr, map[string]bool) Rule
	shape = func(expr ast.Expr, seen map[string]bool) Rule {
		switch t := expr.(type) {
		case *ast.StarExpr:
			r := shape(t.X, seen)
			r.Pointer = true
			return r
		case *ast.ArrayType:
			item := shape(t.Elt, seen)
			return Rule{Kind: "array", Item: &item}
		case *ast.MapType:
			return Rule{Kind: "map"}
		case *ast.InterfaceType:
			return Rule{Kind: "opaque"}
		case *ast.Ident:
			switch t.Name {
			case "string":
				return Rule{Kind: "string"}
			case "bool":
				return Rule{Kind: "boolean"}
			case "int", "uint", "int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64", "float32", "float64":
				return Rule{Kind: "number"}
			}
			if seen[t.Name] {
				panic("recursive input shape " + t.Name)
			}
			decl, ok := types[t.Name]
			if !ok {
				panic("unknown input type " + t.Name)
			}
			seen[t.Name] = true
			r := shape(decl, seen)
			delete(seen, t.Name)
			return r
		case *ast.StructType:
			r := Rule{Kind: "object", Fields: map[string]Rule{}}
			for _, field := range t.Fields.List {
				tag := ""
				if field.Tag != nil {
					raw, e := strconv.Unquote(field.Tag.Value)
					if e != nil {
						panic(e)
					}
					tag = reflect.StructTag(raw).Get("json")
				}
				parts := strings.Split(tag, ",")
				if parts[0] == "-" {
					continue
				}
				rule := shape(field.Type, seen)
				if len(field.Names) == 0 {
					for name, child := range rule.Fields {
						r.Fields[name] = child
					}
					continue
				}
				for _, id := range field.Names {
					if !id.IsExported() {
						continue
					}
					name := parts[0]
					if name == "" {
						name = id.Name
					}
					for _, option := range parts[1:] {
						if option == "omitempty" {
							switch {
							case rule.Pointer:
								rule.Omit = "nil"
							case rule.Kind == "array" || rule.Kind == "map":
								rule.Omit = "empty"
							case rule.Kind == "string" || rule.Kind == "boolean" || rule.Kind == "number":
								rule.Omit = "zero"
							default:
								panic("unsupported omitempty shape " + name)
							}
						}
					}
					r.Fields[name] = rule
				}
			}
			return r
		}
		panic(fmt.Sprintf("unsupported input AST %T", expr))
	}
	for name := range names {
		input, ok := capture.Inputs[name]
		if !ok {
			panic("unmapped tool input " + name)
		}
		if input.Type == "@object" {
			input.Passthrough = "freeform object: original addObjectTool/addConfigSaveTool never projected map entries"
		} else if input.Type == "routing.PreferencesRequest" {
			input.Passthrough = "strict raw preference validation retains its own null/unknown-field policy"
		} else {
			expr, ok := types[input.Type]
			if !ok {
				panic("unknown root " + name + ": " + input.Type)
			}
			rule := shape(expr, map[string]bool{})
			input.Rule = &rule
		}
		capture.Inputs[name] = input
	}
	if len(capture.Inputs) < 100 {
		panic("tool input population collapsed")
	}
	encoded, e := json.MarshalIndent(capture, "", "  ")
	if e != nil {
		panic(e)
	}
	encoded = append(encoded, '\n')
	if *check != "" {
		if !bytes.Equal(encoded, read(*check)) {
			panic("Go typed wire capture changed")
		}
		fmt.Printf("%d tool input contracts match source capture\n", len(capture.Inputs))
		return
	}
	os.Stdout.Write(encoded)
}
