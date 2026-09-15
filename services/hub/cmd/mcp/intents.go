package main

import (
	"context"
	"encoding/json"
	"fmt"

	"github.com/djtouchette/workspacer-hub/internal/busclient"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

// Intent tools use the owner service shared by desktop and headless hosts.
// Keep the action vocabulary closed: the underlying method also exposes human
// verification, publication and lifecycle operations which these tools do not.
const intentMethod = "desktop.intentWorkspaceRequest"

type createIntentIn struct {
	ProjectRoot     string `json:"projectRoot" jsonschema:"absolute project directory on the connected host"`
	Title           string `json:"title" jsonschema:"short human-readable title"`
	Outcome         string `json:"outcome" jsonschema:"desired outcome: what should become possible and for whom"`
	Constraints     string `json:"constraints,omitempty" jsonschema:"requirements, boundaries and out-of-scope work"`
	SuccessCriteria string `json:"successCriteria,omitempty" jsonschema:"one observable acceptance criterion per line"`
	SourceURL       string `json:"sourceUrl,omitempty" jsonschema:"optional reference URL; contents are not fetched"`
}

type listIntentsIn struct {
	ProjectRoot string `json:"projectRoot,omitempty" jsonschema:"filter by the exact projectRoot returned by a previous call; omit for all projects on this host"`
}

type getIntentIn struct {
	ID string `json:"id" jsonschema:"intent ID returned by create_intent or list_intents"`
}

type updateIntentIn struct {
	ID                string  `json:"id" jsonschema:"intent ID"`
	ExpectedRevision  int     `json:"expectedRevision" jsonschema:"revision returned by get_intent; stale writes fail"`
	ExpectedUpdatedAt string  `json:"expectedUpdatedAt" jsonschema:"updatedAt returned by get_intent; guards status-only changes too"`
	Title             *string `json:"title,omitempty" jsonschema:"replacement title; omit to preserve"`
	Outcome           *string `json:"outcome,omitempty" jsonschema:"replacement desired outcome; omit to preserve"`
	Constraints       *string `json:"constraints,omitempty" jsonschema:"replacement constraints; omit to preserve, empty string clears"`
	SuccessCriteria   *string `json:"successCriteria,omitempty" jsonschema:"replacement criteria, one per line; omit to preserve"`
	SourceURL         *string `json:"sourceUrl,omitempty" jsonschema:"replacement reference URL; omit to preserve"`
	Reason            string  `json:"reason" jsonschema:"why the requirements changed"`
}

type addIntentContextIn struct {
	ID               string `json:"id" jsonschema:"intent ID"`
	SourceID         string `json:"sourceId" jsonschema:"caller-generated UUID for this source; reuse with identical content on retry"`
	ExpectedRevision int    `json:"expectedRevision" jsonschema:"current intent revision"`
	Title            string `json:"title" jsonschema:"name of the collected research or reference"`
	Content          string `json:"content" jsonschema:"collected context, findings, file references, decisions and open questions, up to 32000 characters; distinguish observations from assumptions"`
	URL              string `json:"url,omitempty" jsonschema:"optional provenance URL; this tool stores supplied text without fetching or publishing"`
}

func intentCall(ctx context.Context, c *busclient.Client, request any) (json.RawMessage, error) {
	return c.Call(ctx, intentMethod, map[string]any{"request": request})
}

func intentList(ctx context.Context, c *busclient.Client) ([]map[string]any, error) {
	raw, err := intentCall(ctx, c, map[string]any{"action": "list"})
	if err != nil {
		return nil, err
	}
	var result struct {
		Action     string           `json:"action"`
		Workspaces []map[string]any `json:"workspaces"`
	}
	if err := json.Unmarshal(raw, &result); err != nil {
		return nil, err
	}
	if result.Action != "list" || result.Workspaces == nil {
		return nil, fmt.Errorf("unexpected intent list response")
	}
	return result.Workspaces, nil
}

func intentByID(ctx context.Context, c *busclient.Client, id string) (map[string]any, error) {
	if id == "" {
		return nil, fmt.Errorf("intent ID is required")
	}
	rows, err := intentList(ctx, c)
	if err != nil {
		return nil, err
	}
	for _, row := range rows {
		if row["id"] == id {
			return row, nil
		}
	}
	return nil, fmt.Errorf("intent %q no longer exists on this host", id)
}

func addIntentTool[In any](b *build, name, desc string, run func(context.Context, In) (json.RawMessage, error)) {
	if !b.allowed(intentMethod) {
		return
	}
	b.tools = append(b.tools, toolInfo{Name: name, Desc: desc, Method: intentMethod, Group: b.group})
	mcp.AddTool(b.s, &mcp.Tool{Name: name, Description: desc}, func(ctx context.Context, _ *mcp.CallToolRequest, in In) (*mcp.CallToolResult, any, error) {
		raw, err := run(ctx, in)
		if err != nil {
			return &mcp.CallToolResult{IsError: true, Content: []mcp.Content{&mcp.TextContent{Text: err.Error()}}}, nil, nil
		}
		return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: string(raw)}}}, nil, nil
	})
}

type jiraConnectionsIn struct {
	ProjectRoot string `json:"projectRoot" jsonschema:"absolute project directory on the connected host"`
}
type importJiraIntentIn struct {
	ProjectRoot                string `json:"projectRoot" jsonschema:"absolute project directory on the connected host"`
	OperationID                string `json:"operationId" jsonschema:"caller-generated UUID; reuse with identical arguments on retries to avoid duplicate intents"`
	IntegrationID              string `json:"integrationId" jsonschema:"Jira connection ID from list_jira_connections"`
	ExpectedIntegrationVersion int    `json:"expectedIntegrationVersion" jsonschema:"connection version from list_jira_connections"`
	Identifier                 string `json:"identifier" jsonschema:"Jira issue key such as TEAM-123, or its browse URL on the selected Jira site"`
}

func addIntentTools(b *build) {
	addIntentTool(b, "list_jira_connections", "List enabled Jira connections for a project before importing a ticket into an intent.", func(ctx context.Context, in jiraConnectionsIn) (json.RawMessage, error) {
		return intentCall(ctx, b.c, map[string]any{"action": "jiraIntegrations", "projectRoot": in.ProjectRoot})
	})
	addIntentTool(b, "create_intent_from_jira", "Import a Jira ticket as a Draft intent with its original source attached; retries with the same operationId return the same intent.", func(ctx context.Context, in importJiraIntentIn) (json.RawMessage, error) {
		return intentCall(ctx, b.c, map[string]any{"action": "importJiraIntent", "projectRoot": in.ProjectRoot, "operationId": in.OperationID, "integrationId": in.IntegrationID, "expectedIntegrationVersion": in.ExpectedIntegrationVersion, "identifier": in.Identifier})
	})
	addIntentTool(b, "create_intent", "Create a Draft intent on the Work board with collected requirements; returns its ID and revision without launching an agent.", func(ctx context.Context, in createIntentIn) (json.RawMessage, error) {
		return intentCall(ctx, b.c, map[string]any{"action": "create", "projectRoot": in.ProjectRoot, "fields": map[string]any{
			"title": in.Title, "outcome": in.Outcome, "constraints": in.Constraints, "successCriteria": in.SuccessCriteria, "sourceUrl": in.SourceURL, "status": "draft",
		}})
	})
	addIntentTool(b, "list_intents", "List saved intents on the connected host, optionally filtered by project directory.", func(ctx context.Context, in listIntentsIn) (json.RawMessage, error) {
		rows, err := intentList(ctx, b.c)
		if err != nil {
			return nil, err
		}
		filtered := make([]map[string]any, 0, len(rows))
		for _, row := range rows {
			if in.ProjectRoot == "" || row["projectRoot"] == in.ProjectRoot {
				filtered = append(filtered, row)
			}
		}
		return json.Marshal(map[string]any{"intents": filtered})
	})
	addIntentTool(b, "get_intent", "Read one intent's requirements, revision and collected sources before editing or adding context.", func(ctx context.Context, in getIntentIn) (json.RawMessage, error) {
		workspace, err := intentByID(ctx, b.c, in.ID)
		if err != nil {
			return nil, err
		}
		sources, err := intentCall(ctx, b.c, map[string]any{"action": "sources", "id": in.ID})
		if err != nil {
			return nil, err
		}
		return json.Marshal(map[string]any{"workspace": workspace, "sources": sources})
	})
	addIntentTool(b, "update_intent", "Patch intent requirements using revision and timestamp guards; preserves omitted fields and lifecycle status.", func(ctx context.Context, in updateIntentIn) (json.RawMessage, error) {
		if in.ExpectedRevision < 1 || in.ExpectedUpdatedAt == "" {
			return nil, fmt.Errorf("expectedRevision and expectedUpdatedAt from get_intent are required")
		}
		workspace, err := intentByID(ctx, b.c, in.ID)
		if err != nil {
			return nil, err
		}
		if workspace["revision"] != float64(in.ExpectedRevision) || workspace["updatedAt"] != in.ExpectedUpdatedAt {
			return nil, fmt.Errorf("intent changed elsewhere; get_intent again before editing")
		}
		fields := map[string]any{}
		for _, key := range []string{"title", "outcome", "constraints", "successCriteria", "sourceUrl", "status"} {
			fields[key] = workspace[key]
		}
		for key, value := range map[string]*string{"title": in.Title, "outcome": in.Outcome, "constraints": in.Constraints, "successCriteria": in.SuccessCriteria, "sourceUrl": in.SourceURL} {
			if value != nil {
				fields[key] = *value
			}
		}
		return intentCall(ctx, b.c, map[string]any{"action": "update", "id": in.ID, "expectedRevision": in.ExpectedRevision, "expectedUpdatedAt": in.ExpectedUpdatedAt, "fields": fields, "reason": in.Reason})
	})
	addIntentTool(b, "add_intent_context", "Attach collected research as a manual source on an intent; preserves provenance without claiming human verification.", func(ctx context.Context, in addIntentContextIn) (json.RawMessage, error) {
		return intentCall(ctx, b.c, map[string]any{"action": "addSource", "id": in.ID, "sourceId": in.SourceID, "expectedRevision": in.ExpectedRevision, "title": in.Title, "content": in.Content, "connection": map[string]any{"provider": "manual", "url": in.URL, "credentialEnv": ""}})
	})
}
