package api

import (
	"context"
	"net/http"
	"time"

	"github.com/daheige/rsmgo/control/internal/engine"
	"github.com/daheige/rsmgo/control/internal/session"
	pb "github.com/daheige/rsmgo/pb"
	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
)

type Server struct {
	engine          *engine.Client
	sessions        *session.Store
	router          *gin.Engine
	providers       []string
	defaultProvider string
}

func NewServer(engineClient *engine.Client, store *session.Store, providers []string) *Server {
	gin.SetMode(gin.ReleaseMode)
	r := gin.New()
	r.Use(gin.Recovery())
	r.Use(corsMiddleware())

	defaultProvider := "openai"
	if len(providers) > 0 {
		defaultProvider = providers[0]
	}

	s := &Server{
		engine:          engineClient,
		sessions:        store,
		router:          r,
		providers:       providers,
		defaultProvider: defaultProvider,
	}
	s.registerRoutes()
	return s
}

func (s *Server) registerRoutes() {
	s.router.GET("/health", s.health)
	s.router.GET("/api/v1/providers", s.listProviders)
	s.router.GET("/api/v1/models", s.listModels)
	s.router.GET("/api/v1/tools", s.listTools)
	s.router.GET("/api/v1/sessions", s.listSessions)
	s.router.POST("/api/v1/sessions", s.createSession)
	s.router.GET("/api/v1/sessions/:id", s.getSession)
	s.router.POST("/api/v1/sessions/:id/chat", s.chat)
	s.router.DELETE("/api/v1/sessions/:id", s.deleteSession)
}

func (s *Server) Run(addr string) error {
	return s.router.Run(addr)
}

func corsMiddleware() gin.HandlerFunc {
	return func(c *gin.Context) {
		c.Writer.Header().Set("Access-Control-Allow-Origin", "*")
		c.Writer.Header().Set("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS")
		c.Writer.Header().Set("Access-Control-Allow-Headers", "Content-Type, Authorization")
		if c.Request.Method == "OPTIONS" {
			c.AbortWithStatus(http.StatusNoContent)
			return
		}
		c.Next()
	}
}

type healthResponse struct {
	Status    string `json:"status"`
	Version   string `json:"version"`
	Component string `json:"component"`
}

func (s *Server) health(c *gin.Context) {
	ctx, cancel := contextWithTimeout()
	defer cancel()
	engineHealth, err := s.engine.Health(ctx)
	status := "ok"
	if err != nil {
		status = "degraded"
	}
	c.JSON(http.StatusOK, healthResponse{
		Status:    status,
		Version:   engineHealth.GetVersion(),
		Component: "rsmgo-control",
	})
}

func (s *Server) listProviders(c *gin.Context) {
	providers := s.providers
	if providers == nil {
		providers = []string{}
	}
	c.JSON(http.StatusOK, gin.H{"providers": providers})
}

func (s *Server) listModels(c *gin.Context) {
	provider := c.Query("provider")
	ctx, cancel := contextWithTimeout()
	defer cancel()
	resp, err := s.engine.ListModels(ctx, provider)
	if err != nil {
		c.JSON(http.StatusBadGateway, gin.H{"error": err.Error()})
		return
	}
	c.JSON(http.StatusOK, resp)
}

func (s *Server) listTools(c *gin.Context) {
	ctx, cancel := contextWithTimeout()
	defer cancel()
	resp, err := s.engine.ListTools(ctx)
	if err != nil {
		c.JSON(http.StatusBadGateway, gin.H{"error": err.Error()})
		return
	}
	c.JSON(http.StatusOK, resp)
}

func (s *Server) listSessions(c *gin.Context) {
	sessions, err := s.sessions.List()
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"sessions": sessions})
}

type createSessionRequest struct {
	Title    string `json:"title"`
	Provider string `json:"provider"`
	Model    string `json:"model"`
}

func (s *Server) createSession(c *gin.Context) {
	var req createSessionRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}
	if req.Provider == "" {
		req.Provider = s.defaultProvider
	}
	sess := &session.Session{
		ID:       uuid.New().String(),
		Title:    req.Title,
		Provider: req.Provider,
		Model:    req.Model,
	}
	if err := s.sessions.Create(sess); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	c.JSON(http.StatusCreated, sess)
}

func (s *Server) getSession(c *gin.Context) {
	sess, err := s.sessions.Get(c.Param("id"))
	if err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "session not found"})
		return
	}
	c.JSON(http.StatusOK, sess)
}

func (s *Server) deleteSession(c *gin.Context) {
	if err := s.sessions.Delete(c.Param("id")); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"deleted": true})
}

type chatRequest struct {
	Content string `json:"content"`
}

func (s *Server) chat(c *gin.Context) {
	id := c.Param("id")
	sess, err := s.sessions.Get(id)
	if err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "session not found"})
		return
	}
	var req chatRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}

	sess.Messages = append(sess.Messages, session.Message{
		Role:    "user",
		Content: req.Content,
		SentAt:  time.Now().UTC(),
	})

	pbMessages := make([]*pb.Message, 0, len(sess.Messages))
	for _, m := range sess.Messages {
		pbMessages = append(pbMessages, &pb.Message{Role: m.Role, Content: m.Content})
	}

	ctx, cancel := contextWithTimeout(120)
	defer cancel()
	resp, err := s.engine.Chat(ctx, &pb.ChatRequest{
		SessionId: id,
		Messages:  pbMessages,
		Provider:  sess.Provider,
		Model:     sess.Model,
	})
	if err != nil {
		c.JSON(http.StatusBadGateway, gin.H{"error": err.Error()})
		return
	}

	if resp.Message != nil {
		sess.Messages = append(sess.Messages, session.Message{
			Role:    resp.Message.Role,
			Content: resp.Message.Content,
			SentAt:  time.Now().UTC(),
		})
	}
	_ = s.sessions.Update(sess)
	c.JSON(http.StatusOK, resp)
}

func contextWithTimeout(seconds ...int) (context.Context, context.CancelFunc) {
	d := 30
	if len(seconds) > 0 && seconds[0] > 0 {
		d = seconds[0]
	}
	return context.WithTimeout(context.Background(), time.Duration(d)*time.Second)
}
