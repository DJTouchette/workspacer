package taskartifacts

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"reflect"
	"strings"
)

// Decode rejects duplicate keys, case aliases and unknown fields recursively.
// encoding/json's default case-insensitive, last-key-wins behavior is not an
// acceptable interpretation of an immutable plan or an authorization receipt.
func Decode(raw []byte, out any) error {
	if len(raw) > 1<<20 {
		return fmt.Errorf("handoff payload exceeds wire limit")
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	if err := validateValue(d, reflect.TypeOf(out).Elem()); err != nil {
		return err
	}
	if _, err := d.Token(); err != io.EOF {
		return fmt.Errorf("trailing handoff payload")
	}
	return json.Unmarshal(raw, out)
}

// CheckJSON runs before a router rewrites identity fields; otherwise decoding
// into a map would already have erased duplicate-key evidence.
func CheckJSON(raw []byte) error {
	if len(raw) > 1<<20 {
		return fmt.Errorf("handoff payload exceeds wire limit")
	}
	d := json.NewDecoder(bytes.NewReader(raw))
	var walk func(int) error
	walk = func(depth int) error {
		if depth > 16 {
			return fmt.Errorf("handoff nesting exceeds limit")
		}
		t, err := d.Token()
		if err != nil {
			return err
		}
		if delim, ok := t.(json.Delim); ok {
			switch delim {
			case '{':
				seen := map[string]bool{}
				for d.More() {
					key, err := d.Token()
					if err != nil {
						return err
					}
					name, ok := key.(string)
					if !ok || seen[strings.ToLower(name)] {
						return fmt.Errorf("duplicate or aliased handoff key")
					}
					seen[strings.ToLower(name)] = true
					if err := walk(depth + 1); err != nil {
						return err
					}
				}
			case '[':
				for d.More() {
					if err := walk(depth + 1); err != nil {
						return err
					}
				}
			default:
				return fmt.Errorf("invalid handoff JSON")
			}
			_, err := d.Token()
			return err
		}
		return nil
	}
	if err := walk(0); err != nil {
		return err
	}
	if _, err := d.Token(); err != io.EOF {
		return fmt.Errorf("trailing handoff JSON")
	}
	return nil
}

func validateValue(d *json.Decoder, typ reflect.Type) error {
	for typ.Kind() == reflect.Pointer {
		typ = typ.Elem()
	}
	token, err := d.Token()
	if err != nil {
		return err
	}
	if token == nil {
		return nil
	}
	switch delim := token.(type) {
	case json.Delim:
		if delim == '{' {
			if typ.Kind() != reflect.Struct {
				return fmt.Errorf("unexpected handoff object")
			}
			fields := map[string]reflect.Type{}
			for i := 0; i < typ.NumField(); i++ {
				f := typ.Field(i)
				fields[strings.Split(f.Tag.Get("json"), ",")[0]] = f.Type
			}
			seen := map[string]bool{}
			for d.More() {
				key, err := d.Token()
				if err != nil {
					return err
				}
				name, ok := key.(string)
				if !ok {
					return fmt.Errorf("invalid handoff key")
				}
				field, ok := fields[name]
				if !ok || seen[name] {
					return fmt.Errorf("unknown, aliased or duplicate handoff field: %s", name)
				}
				seen[name] = true
				if err := validateValue(d, field); err != nil {
					return err
				}
			}
			_, err := d.Token()
			return err
		}
		if delim == '[' {
			if typ.Kind() != reflect.Slice {
				return fmt.Errorf("unexpected handoff array")
			}
			for d.More() {
				if err := validateValue(d, typ.Elem()); err != nil {
					return err
				}
			}
			_, err := d.Token()
			return err
		}
		return fmt.Errorf("invalid handoff delimiter")
	}
	return nil
}
