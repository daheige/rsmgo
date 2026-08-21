package engine

import (
	"context"
	"fmt"

	pb "github.com/daheige/rsmgo/pb"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

type Client struct {
	conn   *grpc.ClientConn
	engine pb.EngineClient
}

func NewClient(addr string) (*Client, error) {
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, fmt.Errorf("dial engine: %w", err)
	}
	return &Client{
		conn:   conn,
		engine: pb.NewEngineClient(conn),
	}, nil
}

func (c *Client) Close() error {
	return c.conn.Close()
}

func (c *Client) Health(ctx context.Context) (*pb.HealthResponse, error) {
	return c.engine.Health(ctx, &pb.HealthRequest{})
}

func (c *Client) Chat(ctx context.Context, req *pb.ChatRequest) (*pb.ChatResponse, error) {
	return c.engine.Chat(ctx, req)
}

func (c *Client) ListTools(ctx context.Context) (*pb.ListToolsResponse, error) {
	return c.engine.ListTools(ctx, &pb.ListToolsRequest{})
}

func (c *Client) ListModels(ctx context.Context, provider string) (*pb.ListModelsResponse, error) {
	return c.engine.ListModels(ctx, &pb.ListModelsRequest{Provider: provider})
}

func (c *Client) ExecuteTool(ctx context.Context, name, args string) (*pb.ExecuteToolResponse, error) {
	return c.engine.ExecuteTool(ctx, &pb.ExecuteToolRequest{Name: name, Arguments: args})
}
