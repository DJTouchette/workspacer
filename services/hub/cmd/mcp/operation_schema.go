package main

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/google/jsonschema-go/jsonschema"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Keep one wire struct without charging every operation for unrelated fields.
// A misspelled registry field is a programming error, caught at registration.
func operationSchema[In any](fields, required string) *jsonschema.Schema {
	schema, err := jsonschema.For[In](nil)
	if err != nil {
		panic(err)
	}
	properties := map[string]*jsonschema.Schema{}
	for _, field := range strings.Fields(fields) {
		property, ok := schema.Properties[field]
		if !ok {
			panic("unknown operation field: " + field)
		}
		properties[field] = property
	}
	schema.Properties = properties
	schema.Required = strings.Fields(required)
	for _, field := range schema.Required {
		if properties[field] == nil {
			panic("required field not exposed: " + field)
		}
	}
	return schema
}

// Opt-in projection: the host still owns policy and validates the full pinned
// template. Only its body is omitted on the return path, never result evidence.
func forwardWorkflow(ctx context.Context, b *build, method string, wire any, compact bool) (*mcp.CallToolResult, any, error) {
	result, extra, err := b.forward(ctx, method, wire)
	if !compact || err != nil || result == nil || result.IsError {
		return result, extra, err
	}
	for _, content := range result.Content {
		text, ok := content.(*mcp.TextContent)
		if !ok {
			continue
		}
		var value map[string]any
		decoder := json.NewDecoder(strings.NewReader(text.Text))
		decoder.UseNumber() // Projection must not round integers in outcome evidence.
		if !json.Valid([]byte(text.Text)) || decoder.Decode(&value) != nil {
			continue
		}
		omitted := false
		strip := func(value any) {
			task, _ := value.(map[string]any)
			workflow, _ := task["workflow"].(map[string]any)
			templates, _ := workflow["templates"].(map[string]any)
			for _, value := range templates {
				template, _ := value.(map[string]any)
				if _, exists := template["body"]; exists {
					delete(template, "body")
					omitted = true
				}
			}
		}
		strip(value["task"])
		if tasks, ok := value["tasks"].([]any); ok {
			for _, task := range tasks {
				strip(task)
			}
		}
		if omitted {
			value["omitted"] = []string{"workflow template bodies; use next_workflow_step with compact:false for the full pinned task"}
			if raw, err := json.Marshal(value); err == nil {
				text.Text = string(raw)
			}
		}
	}
	return result, extra, err
}
