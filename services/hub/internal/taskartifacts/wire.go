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
