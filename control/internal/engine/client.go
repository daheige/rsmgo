package engine

import (
	"context"
	"fmt"
	"time"

	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"

	pb "github.com/daheige/rsmgo/pb"
)

type Client struct {
	conn   *grpc.ClientConn
	engine pb.EngineClient
}

// NewClient create grpc client
func NewClient(addr string) (*Client, error) {
	conn, err := grpc.NewClient(addr,
		grpc.WithDefaultServiceConfig(`{"loadBalancingConfig": [{"round_robin":{}}]}`),
		grpc.WithTransportCredentials(insecure.NewCredentials()),
		grpc.WithIdleTimeout(30*time.Minute), // 连接生命周期
		grpc.WithMaxCallAttempts(3),          // 最大重试次数
	)
	if err != nil {
		return nil, fmt.Errorf("dial engine error: %w", err)
	}

	return &Client{
		conn:   conn,
		engine: pb.NewEngineClient(conn),
	}, nil
}

// Close client close
func (c *Client) Close() error {
	return c.conn.Close()
}

// Health client healthz check
func (c *Client) Health(ctx context.Context) (*pb.HealthResponse, error) {
	return c.engine.Health(ctx, &pb.HealthRequest{})
}

// Chat call llm provider chat,eg: deepseek chat
func (c *Client) Chat(ctx context.Context, req *pb.ChatRequest) (*pb.ChatResponse, error) {
	return c.engine.Chat(ctx, req)
}

// ChatStream call llm provider use chat stream
func (c *Client) ChatStream(ctx context.Context, req *pb.ChatRequest) (pb.Engine_ChatStreamClient, error) {
	return c.engine.ChatStream(ctx, req)
}

// ListTools tools list
func (c *Client) ListTools(ctx context.Context) (*pb.ListToolsResponse, error) {
	return c.engine.ListTools(ctx, &pb.ListToolsRequest{})
}

// ListModels models list
func (c *Client) ListModels(ctx context.Context, provider string) (*pb.ListModelsResponse, error) {
	return c.engine.ListModels(ctx, &pb.ListModelsRequest{Provider: provider})
}

// ExecuteTool call functions tool
func (c *Client) ExecuteTool(ctx context.Context, name, args string) (*pb.ExecuteToolResponse, error) {
	return c.engine.ExecuteTool(ctx, &pb.ExecuteToolRequest{Name: name, Arguments: args})
}
