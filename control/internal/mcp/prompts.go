package mcp

import (
	"context"
	"fmt"

	mcpgo "github.com/modelcontextprotocol/go-sdk/mcp"
)

// userMessage is a small helper for building a single-user-message prompt
// result; MCP has no system role, so instructions travel as a user message.
func userMessage(text string) *mcpgo.GetPromptResult {
	return &mcpgo.GetPromptResult{
		Messages: []*mcpgo.PromptMessage{{
			Role:    mcpgo.Role("user"),
			Content: &mcpgo.TextContent{Text: text},
		}},
	}
}

// registerPrompts installs the built-in prompt templates the rsmgo MCP server
// offers to clients. Each template takes plain string arguments and returns
// ready-to-send messages, so clients like Claude Desktop can surface them as
// slash-command style helpers.
func registerPrompts(srv *mcpgo.Server) {
	srv.AddPrompt(&mcpgo.Prompt{
		Name:        "code_review",
		Title:       "Code Review",
		Description: "Review code for correctness, security, performance, and readability issues.",
		Arguments: []*mcpgo.PromptArgument{
			{Name: "code", Description: "The source code to review", Required: true},
			{Name: "language", Description: "Programming language of the code (e.g. rust, go)"},
		},
	}, func(_ context.Context, req *mcpgo.GetPromptRequest) (*mcpgo.GetPromptResult, error) {
		args := req.Params.Arguments
		language := args["language"]
		if language == "" {
			language = "text"
		}
		return userMessage(fmt.Sprintf(`You are a senior software engineer performing a code review.
Review the following %s code. List concrete problems first, ordered by severity (correctness bugs, security vulnerabilities, performance issues, then readability), and end with suggested improvements.

`+"```"+`%s
%s
`+"```", language, language, args["code"])), nil
	})

	srv.AddPrompt(&mcpgo.Prompt{
		Name:        "explain_code",
		Title:       "Explain Code",
		Description: "Explain what a piece of code does, line by line when helpful.",
		Arguments: []*mcpgo.PromptArgument{
			{Name: "code", Description: "The source code to explain", Required: true},
			{Name: "language", Description: "Programming language of the code"},
		},
	}, func(_ context.Context, req *mcpgo.GetPromptRequest) (*mcpgo.GetPromptResult, error) {
		args := req.Params.Arguments
		language := args["language"]
		if language == "" {
			language = "text"
		}
		return userMessage(fmt.Sprintf("Explain what the following %s code does, in clear prose. Start with a one-paragraph summary, then walk through the important parts.\n\n```%s\n%s\n```", language, language, args["code"])), nil
	})

	srv.AddPrompt(&mcpgo.Prompt{
		Name:        "summarize_text",
		Title:       "Summarize Text",
		Description: "Summarize a text into key points.",
		Arguments: []*mcpgo.PromptArgument{
			{Name: "text", Description: "The text to summarize", Required: true},
		},
	}, func(_ context.Context, req *mcpgo.GetPromptRequest) (*mcpgo.GetPromptResult, error) {
		return userMessage("Summarize the following text as a bulleted list of key points, then add a one-sentence takeaway.\n\n" + req.Params.Arguments["text"]), nil
	})
}
