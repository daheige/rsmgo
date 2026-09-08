package mcp

import (
	"context"
	"encoding/json"

	mcpgo "github.com/modelcontextprotocol/go-sdk/mcp"
)

// registerResources exposes live engine state as MCP resources under the
// rsmgo:// URI scheme. Values are recomputed on every read, so they always
// reflect the current engine rather than a snapshot from startup.
func registerResources(srv *mcpgo.Server, ec EngineClient) {
	add := func(uri, name, description string, handler mcpgo.ResourceHandler) {
		srv.AddResource(&mcpgo.Resource{
			URI:         uri,
			Name:        name,
			Description: description,
			MIMEType:    "application/json",
		}, handler)
	}

	add("rsmgo://tools", "tools",
		"The engine's current tool list, including tools imported from external MCP servers. Recomputed on every read.",
		func(ctx context.Context, req *mcpgo.ReadResourceRequest) (*mcpgo.ReadResourceResult, error) {
			resp, err := ec.ListTools(ctx)
			if err != nil {
				return nil, err
			}
			type toolEntry struct {
				Name             string          `json:"name"`
				Description      string          `json:"description"`
				ParametersSchema json.RawMessage `json:"parameters_schema"`
			}
			entries := make([]toolEntry, 0, len(resp.Tools))
			for _, t := range resp.Tools {
				entries = append(entries, toolEntry{
					Name:             t.Name,
					Description:      t.Description,
					ParametersSchema: json.RawMessage(t.ParametersSchema),
				})
			}
			return jsonResource(req.Params.URI, entries)
		})

	add("rsmgo://providers", "providers",
		"The configured LLM providers and their models.",
		func(ctx context.Context, req *mcpgo.ReadResourceRequest) (*mcpgo.ReadResourceResult, error) {
			resp, err := ec.ListModels(ctx, "")
			if err != nil {
				return nil, err
			}
			return jsonResource(req.Params.URI, resp.Models)
		})

	add("rsmgo://health", "health",
		"Engine health status and version.",
		func(ctx context.Context, req *mcpgo.ReadResourceRequest) (*mcpgo.ReadResourceResult, error) {
			resp, err := ec.Health(ctx)
			if err != nil {
				return nil, err
			}
			return jsonResource(req.Params.URI, resp)
		})
}

// jsonResource builds a ReadResourceResult holding one JSON text document.
func jsonResource(uri string, v any) (*mcpgo.ReadResourceResult, error) {
	data, err := json.MarshalIndent(v, "", "  ")
	if err != nil {
		return nil, err
	}
	return &mcpgo.ReadResourceResult{
		Contents: []*mcpgo.ResourceContents{{
			URI:      uri,
			MIMEType: "application/json",
			Text:     string(data),
		}},
	}, nil
}
