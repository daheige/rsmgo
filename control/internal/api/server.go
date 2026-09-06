package api

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"html"
	"io"
	"log"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/daheige/rsmgo/control/internal/engine"
	"github.com/daheige/rsmgo/control/internal/session"
	"github.com/daheige/rsmgo/control/internal/workspace"
	pb "github.com/daheige/rsmgo/pb"
	"github.com/gin-gonic/gin"
	"github.com/google/uuid"
	"github.com/ledongthuc/pdf"
)

// maxImageBytes caps the size of an image attachment that is sent to the model
// as base64 multimodal content. Larger images are skipped to avoid oversized
// requests (the model still sees an "[Image attached: name]" note).
const maxImageBytes = 10 * 1024 * 1024

type Server struct {
	engine          *engine.Client
	sessions        *session.Store
	workspaces      *workspace.Store
	router          *gin.Engine
	providers       []string
	defaultProvider string
	uploadDir       string
	outputsDir      string
	chatStream      bool
	// activeChats maps session id -> context cancel func for in-flight chat
	// requests, so a user can stop generation.
	activeChats sync.Map
}

func NewServer(engineClient *engine.Client, store *session.Store, providers []string, dataDir string, chatStream bool) *Server {
	gin.SetMode(gin.ReleaseMode)
	r := gin.New()
	r.Use(gin.Recovery())
	r.Use(corsMiddleware())
	r.Use(requestLogger())

	defaultProvider := "openai"
	if len(providers) > 0 {
		defaultProvider = providers[0]
	}

	uploadDir := filepath.Join(dataDir, "uploads")
	outputsDir := filepath.Join(dataDir, "outputs")
	_ = os.MkdirAll(uploadDir, 0o755)
	_ = os.MkdirAll(outputsDir, 0o755)

	workspaceStore := workspace.NewStore(filepath.Join(dataDir, "workspaces"))

	s := &Server{
		engine:          engineClient,
		sessions:        store,
		workspaces:      workspaceStore,
		router:          r,
		providers:       providers,
		defaultProvider: defaultProvider,
		uploadDir:       uploadDir,
		outputsDir:      outputsDir,
		chatStream:      chatStream,
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
	s.router.PATCH("/api/v1/sessions/:id", s.updateSession)
	s.router.POST("/api/v1/sessions/:id/chat", s.chat)
	s.router.POST("/api/v1/sessions/:id/chat/cancel", s.cancelChat)
	s.router.DELETE("/api/v1/sessions/:id", s.deleteSession)
	s.router.POST("/api/v1/uploads", s.uploadFile)
	s.router.GET("/api/v1/uploads/:id", s.downloadFile)
	s.router.GET("/api/v1/files/:name", s.downloadOutputFile)
	// Compatibility redirect for models that emit the unversioned /api/files/ path.
	s.router.GET("/api/files/:name", s.redirectOutputFile)
	s.router.GET("/api/v1/workspaces", s.listWorkspaces)
	s.router.POST("/api/v1/workspaces", s.createWorkspace)
	s.router.DELETE("/api/v1/workspaces/:id", s.deleteWorkspace)
	s.router.GET("/api/v1/workspaces/:id/files/:name", s.downloadWorkspaceFile)
}

// MountMCP attaches a Streamable HTTP MCP endpoint at /mcp, exposing the
// engine's tool set to external MCP clients.
func (s *Server) MountMCP(handler http.Handler) {
	s.router.Any("/mcp", gin.WrapH(handler))
}

func (s *Server) Run(addr string) error {
	// Use a custom http.Server with generous timeouts so long-running chat
	// requests (model API calls, tool execution loops) are not cut off by the
	// control plane before the engine responds.
	server := &http.Server{
		Addr:         addr,
		Handler:      s.router,
		ReadTimeout:  5 * time.Minute,
		WriteTimeout: 5 * time.Minute,
		IdleTimeout:  2 * time.Minute,
	}

	return server.ListenAndServe()
}

func corsMiddleware() gin.HandlerFunc {
	return func(c *gin.Context) {
		c.Writer.Header().Set("Access-Control-Allow-Origin", "*")
		c.Writer.Header().Set("Access-Control-Allow-Methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS")
		c.Writer.Header().Set("Access-Control-Allow-Headers", "Content-Type, Authorization")
		if c.Request.Method == "OPTIONS" {
			c.AbortWithStatus(http.StatusNoContent)
			return
		}
		c.Next()
	}
}

// requestLogger logs each HTTP request: client IP, method, path, status code,
// and latency. /health is skipped to keep polling noise out of the log.
func requestLogger() gin.HandlerFunc {
	return func(c *gin.Context) {
		if c.Request.URL.Path == "/health" {
			c.Next()
			return
		}
		start := time.Now()
		c.Next()
		log.Printf("%s %s %s -> %d (%s)\n",
			c.ClientIP(), c.Request.Method, c.Request.URL.Path,
			c.Writer.Status(), time.Since(start).Round(time.Millisecond))
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
	Title       string `json:"title"`
	Provider    string `json:"provider"`
	Model       string `json:"model"`
	WorkspaceID string `json:"workspace_id"`
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
	if req.WorkspaceID != "" {
		if _, err := s.workspaces.Get(req.WorkspaceID); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "workspace not found"})
			return
		}
	}
	sess := &session.Session{
		ID:          uuid.New().String(),
		Title:       req.Title,
		Provider:    req.Provider,
		Model:       req.Model,
		WorkspaceID: req.WorkspaceID,
	}
	if err := s.sessions.Create(sess); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	log.Printf("session created id=%s provider=%s model=%q", sess.ID, sess.Provider, sess.Model)
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
	id := c.Param("id")
	if err := s.sessions.Delete(id); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	log.Printf("session deleted id=%s", id)
	c.JSON(http.StatusOK, gin.H{"deleted": true})
}

type updateSessionRequest struct {
	Title       *string `json:"title"`
	Pinned      *bool   `json:"pinned"`
	WorkspaceID *string `json:"workspace_id"`
}

func (s *Server) updateSession(c *gin.Context) {
	id := c.Param("id")
	var req updateSessionRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}
	if req.Title == nil && req.Pinned == nil && req.WorkspaceID == nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "nothing to update"})
		return
	}

	if req.WorkspaceID != nil && *req.WorkspaceID != "" {
		if _, err := s.workspaces.Get(*req.WorkspaceID); err != nil {
			c.JSON(http.StatusBadRequest, gin.H{"error": "workspace not found"})
			return
		}
	}

	sess, err := s.sessions.Patch(id, func(s *session.Session) error {
		if req.Title != nil {
			s.Title = strings.TrimSpace(*req.Title)
			if s.Title == "" {
				s.Title = "New chat"
			}
		}
		if req.Pinned != nil {
			s.Pinned = *req.Pinned
		}
		if req.WorkspaceID != nil {
			s.WorkspaceID = strings.TrimSpace(*req.WorkspaceID)
		}
		return nil
	})
	if err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "session not found"})
		return
	}
	c.JSON(http.StatusOK, sess)
}

type chatRequest struct {
	Content       string   `json:"content"`
	ToolNames     []string `json:"tool_names"`
	WebSearch     bool     `json:"web_search"`
	AttachmentIDs []string `json:"attachment_ids"`
	Regenerate    bool     `json:"regenerate"`
	Stream        bool     `json:"stream"`
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

	content := s.withAttachments(req.Content, req.AttachmentIDs)
	imageParts := s.imageParts(req.AttachmentIDs)

	// Resolve the session's workspace so the engine can scope file tools to it.
	ws := s.workspaceFor(sess.WorkspaceID)
	var workspacePath, workspaceID string
	if ws != nil {
		workspacePath = ws.Path
		workspaceID = ws.ID
	}

	if req.Regenerate {
		// Regenerate: drop any trailing assistant message(s) so the model
		// re-answers the last user prompt instead of appending a new one.
		for len(sess.Messages) > 0 && sess.Messages[len(sess.Messages)-1].Role == "assistant" {
			sess.Messages = sess.Messages[:len(sess.Messages)-1]
		}
	} else {
		sess.Messages = append(sess.Messages, session.Message{
			Role:    "user",
			Content: content,
			SentAt:  time.Now().UTC(),
		})
	}
	// Persist the user message up front so it survives a cancellation.
	_ = s.sessions.Update(sess)

	pbMessages := make([]*pb.Message, 0, len(sess.Messages))
	for i, m := range sess.Messages {
		pm := &pb.Message{Role: m.Role, Content: m.Content}
		if i == len(sess.Messages)-1 && len(imageParts) > 0 {
			pm.Parts = imageParts
		}
		pbMessages = append(pbMessages, pm)
	}

	toolNames := append([]string{}, req.ToolNames...)
	if req.WebSearch && !contains(toolNames, "web_search") {
		toolNames = append(toolNames, "web_search")
	}
	// Enforce workspace-level tool permissions: when the workspace restricts the
	// tool set, drop any requested tool that is not explicitly allowed.
	if ws != nil && len(ws.Tools) > 0 {
		toolNames = intersectTools(toolNames, ws.Tools)
	}

	log.Printf("chat start session=%s provider=%s model=%q workspace=%q tools=%v",
		id, sess.Provider, sess.Model, workspaceID, toolNames)
	started := time.Now()

	// Cancellable context: aborted when the client disconnects or when the
	// user hits the stop button (which calls the cancel endpoint).
	ctx, cancel := context.WithCancel(c.Request.Context())
	s.activeChats.Store(id, cancel)
	defer func() {
		s.activeChats.Delete(id)
		cancel()
	}()
	timeout := 120 * time.Second
	if req.Stream {
		timeout = 5 * time.Minute
	}
	ctx, timeoutCancel := context.WithTimeout(ctx, timeout)
	defer timeoutCancel()

	pbReq := &pb.ChatRequest{
		SessionId:   id,
		Messages:    pbMessages,
		Provider:    sess.Provider,
		Model:       sess.Model,
		ToolNames:   toolNames,
		Workspace:   workspacePath,
		WorkspaceId: workspaceID,
	}

	if req.Stream && s.chatStream {
		s.streamChat(c, ctx, id, sess, pbReq, started)
		return
	}

	resp, err := s.engine.Chat(ctx, pbReq)
	if err != nil {
		if ctx.Err() == context.Canceled {
			log.Printf("chat cancelled session=%s after=%s", id, time.Since(started).Round(time.Millisecond))
			if req.Stream {
				sseError(c.Writer, "cancelled")
				return
			}
			c.JSON(http.StatusOK, gin.H{"cancelled": true})
			return
		}
		log.Printf("chat failed session=%s: %v", id, err)
		if req.Stream {
			sseError(c.Writer, err.Error())
			return
		}
		c.JSON(http.StatusBadGateway, gin.H{"error": err.Error()})
		return
	}

	if req.Stream {
		s.streamSingle(c, id, sess, resp, started)
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
	log.Printf("chat done session=%s after=%s", id, time.Since(started).Round(time.Millisecond))
	c.JSON(http.StatusOK, resp)
}

// streamSingle writes a single non-streaming ChatResponse to an SSE stream.
// Used when the client requested streaming but the server has disabled it via
// the chat_stream configuration option.
func (s *Server) streamSingle(c *gin.Context, id string, sess *session.Session, resp *pb.ChatResponse, started time.Time) {
	c.Writer.Header().Set("Content-Type", "text/event-stream")
	c.Writer.Header().Set("Cache-Control", "no-cache")
	c.Writer.Header().Set("Connection", "keep-alive")
	c.Writer.Header().Set("X-Accel-Buffering", "no")
	c.Writer.WriteHeader(http.StatusOK)

	if resp.Message != nil {
		sess.Messages = append(sess.Messages, session.Message{
			Role:    resp.Message.Role,
			Content: resp.Message.Content,
			SentAt:  time.Now().UTC(),
		})
		_ = s.sessions.Update(sess)
	}

	chunk := &pb.ChatStreamChunk{
		SessionId: id,
		Delta:     "",
		Done:      true,
		Message:   resp.Message,
		ToolCalls: resp.ToolCalls,
	}
	data, _ := json.Marshal(chunk)
	_, _ = c.Writer.WriteString("data: ")
	_, _ = c.Writer.Write(data)
	_, _ = c.Writer.WriteString("\n\n")
	c.Writer.Flush()
	log.Printf("chat buffered stream done session=%s after=%s", id, time.Since(started).Round(time.Millisecond))
}

// streamChat relays the engine's ChatStream gRPC stream to the client as
// Server-Sent Events: each chunk is marshalled to JSON and written as a
// `data:` line, with the final assistant message persisted to the session when
// the stream reports done.
func (s *Server) streamChat(c *gin.Context, ctx context.Context, id string, sess *session.Session, pbReq *pb.ChatRequest, started time.Time) {
	c.Writer.Header().Set("Content-Type", "text/event-stream")
	c.Writer.Header().Set("Cache-Control", "no-cache")
	c.Writer.Header().Set("Connection", "keep-alive")
	c.Writer.Header().Set("X-Accel-Buffering", "no")
	c.Writer.WriteHeader(http.StatusOK)

	stream, err := s.engine.ChatStream(ctx, pbReq)
	if err != nil {
		log.Printf("chat stream open failed session=%s: %v", id, err)
		sseError(c.Writer, err.Error())
		return
	}

	for {
		chunk, err := stream.Recv()
		if err == io.EOF {
			break
		}
		if err != nil {
			if ctx.Err() == context.Canceled {
				log.Printf("chat stream cancelled session=%s after=%s", id, time.Since(started).Round(time.Millisecond))
				return
			}
			log.Printf("chat stream failed session=%s: %v", id, err)
			sseError(c.Writer, err.Error())
			return
		}

		data, err := json.Marshal(chunk)
		if err != nil {
			sseError(c.Writer, err.Error())
			return
		}
		if _, err := c.Writer.WriteString("data: "); err != nil {
			return
		}
		if _, err := c.Writer.Write(data); err != nil {
			return
		}
		if _, err := c.Writer.WriteString("\n\n"); err != nil {
			return
		}
		c.Writer.Flush()

		if chunk.Done && chunk.Message != nil {
			sess.Messages = append(sess.Messages, session.Message{
				Role:    chunk.Message.Role,
				Content: chunk.Message.Content,
				SentAt:  time.Now().UTC(),
			})
			_ = s.sessions.Update(sess)
		}
	}
	log.Printf("chat stream done session=%s after=%s", id, time.Since(started).Round(time.Millisecond))
}

// sseError writes a single SSE error event and flushes it to the client. Used
// to report a stream failure after the response has already started.
func sseError(w gin.ResponseWriter, msg string) {
	data, _ := json.Marshal(gin.H{"error": msg})
	_, _ = w.WriteString("data: ")
	_, _ = w.Write(data)
	_, _ = w.WriteString("\n\n")
	w.Flush()
}

// workspaceFor resolves a workspace id into its workspace record. An unknown or
// empty id yields nil, in which case the engine falls back to the default
// outputs directory and no tool restrictions apply.
func (s *Server) workspaceFor(workspaceID string) *workspace.Workspace {
	if workspaceID == "" {
		return nil
	}
	ws, err := s.workspaces.Get(workspaceID)
	if err != nil {
		return nil
	}
	return ws
}

// intersectTools keeps the requested tools that are also present in the allowed
// set, preserving the requested order.
func intersectTools(requested, allowed []string) []string {
	out := make([]string, 0, len(requested))
	for _, t := range requested {
		if contains(allowed, t) {
			out = append(out, t)
		}
	}
	return out
}

func contains(items []string, target string) bool {
	for _, it := range items {
		if it == target {
			return true
		}
	}
	return false
}

// attachmentMeta describes a stored upload. The file bytes live at
// uploadDir/<id> and the metadata at uploadDir/<id>.json.
type attachmentMeta struct {
	Name        string `json:"name"`
	ContentType string `json:"content_type"`
	Size        int64  `json:"size"`
}

func (s *Server) uploadFile(c *gin.Context) {
	file, header, err := c.Request.FormFile("file")
	if err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "expected multipart file field 'file'"})
		return
	}
	defer file.Close()

	id := uuid.New().String()
	data, err := io.ReadAll(file)
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}

	contentType := header.Header.Get("Content-Type")
	if contentType == "" {
		contentType = http.DetectContentType(data)
	}
	meta := attachmentMeta{
		Name:        header.Filename,
		ContentType: contentType,
		Size:        int64(len(data)),
	}
	metaBytes, _ := json.Marshal(meta)

	if err := os.WriteFile(filepath.Join(s.uploadDir, id), data, 0o644); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	if err := os.WriteFile(filepath.Join(s.uploadDir, id+".json"), metaBytes, 0o644); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}

	log.Printf("upload id=%s name=%q size=%d type=%s", id, meta.Name, meta.Size, meta.ContentType)
	c.JSON(http.StatusCreated, gin.H{
		"id":           id,
		"name":         meta.Name,
		"content_type": meta.ContentType,
		"size":         meta.Size,
	})
}

func (s *Server) downloadFile(c *gin.Context) {
	id := c.Param("id")
	meta, data, err := s.loadAttachment(id)
	if err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "attachment not found"})
		return
	}
	contentType := meta.ContentType
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	c.Data(http.StatusOK, contentType, data)
}

// redirectOutputFile redirects the legacy unversioned /api/files/:name path to
// the canonical /api/v1/files/:name endpoint. Some models emit the shorter
// path in their final response, so this keeps those links working.
func (s *Server) redirectOutputFile(c *gin.Context) {
	name := c.Param("name")
	c.Redirect(http.StatusMovedPermanently, "/api/v1/files/"+name)
}

// downloadOutputFile serves files written by the write_file tool from the
// workspace outputs directory. The file name is restricted to a simple base
// name to prevent directory traversal.
func (s *Server) downloadOutputFile(c *gin.Context) {
	name := c.Param("name")
	name = filepath.Base(name)
	if decoded, err := url.PathUnescape(name); err == nil {
		name = decoded
	}
	if name == "" || name == "." || name == "/" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "invalid file name"})
		return
	}
	path := filepath.Join(s.outputsDir, name)
	data, err := os.ReadFile(path)
	if err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "file not found"})
		return
	}
	contentType := http.DetectContentType(data)
	c.Header("Content-Disposition", "attachment; filename=\""+name+"\"")
	c.Data(http.StatusOK, contentType, data)
}

func (s *Server) loadAttachment(id string) (attachmentMeta, []byte, error) {
	metaBytes, err := os.ReadFile(filepath.Join(s.uploadDir, id+".json"))
	if err != nil {
		return attachmentMeta{}, nil, err
	}
	var meta attachmentMeta
	if err := json.Unmarshal(metaBytes, &meta); err != nil {
		return attachmentMeta{}, nil, err
	}
	data, err := os.ReadFile(filepath.Join(s.uploadDir, id))
	if err != nil {
		return attachmentMeta{}, nil, err
	}
	return meta, data, nil
}

// withAttachments appends a readable representation of each attachment to the
// user's message so the model can see it. Text-like files are inlined (up to a
// cap); documents (PDF/DOCX) have their text extracted; binary files are
// described by name and size; images are referenced by name (their pixels are
// sent separately as multimodal image parts).
func (s *Server) withAttachments(content string, ids []string) string {
	if len(ids) == 0 {
		return content
	}
	var b strings.Builder
	b.WriteString(content)
	for _, id := range ids {
		meta, data, err := s.loadAttachment(id)
		if err != nil {
			continue
		}
		if isImageContent(meta.ContentType) {
			b.WriteString("\n\n[Image attached: ")
			b.WriteString(meta.Name)
			b.WriteString("]")
			continue
		}
		b.WriteString("\n\n[Attached file: ")
		b.WriteString(meta.Name)
		b.WriteString("]")
		var text string
		var found bool
		switch {
		case isTextContent(meta.ContentType) || looksText(data):
			text, found = string(data), true
		default:
			text, found = extractDocumentText(data)
		}
		if found {
			b.WriteString("\n```\n")
			b.WriteString(truncateText(text, 8000))
			b.WriteString("\n```")
		} else {
			b.WriteString(" (binary file, ")
			b.WriteString(strconv.FormatInt(meta.Size, 10))
			b.WriteString(" bytes)")
		}
	}
	return b.String()
}

// truncateText limits text to capBytes without splitting a UTF-8 rune.
func truncateText(text string, capBytes int) string {
	if len(text) <= capBytes {
		return text
	}
	cut := capBytes
	for cut > 0 && !utf8.RuneStart(text[cut]) {
		cut--
	}
	return text[:cut] + "\n...(truncated)"
}

// extractDocumentText pulls readable text out of common document formats
// (PDF, DOCX). Returns the extracted text and true on success.
func extractDocumentText(data []byte) (string, bool) {
	if len(data) >= 5 && string(data[:5]) == "%PDF-" {
		return extractPdfText(data)
	}
	// DOCX (and other OOXML files) are ZIP archives starting with "PK".
	if len(data) >= 4 && string(data[:4]) == "PK\x03\x04" {
		return extractDocxText(data)
	}
	return "", false
}

func extractPdfText(data []byte) (string, bool) {
	r, err := pdf.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return "", false
	}
	textReader, err := r.GetPlainText()
	if err != nil {
		return "", false
	}
	var buf bytes.Buffer
	if _, err := buf.ReadFrom(textReader); err != nil {
		return "", false
	}
	text := strings.TrimSpace(buf.String())
	if text == "" {
		return "", false
	}
	return text, true
}

func extractDocxText(data []byte) (string, bool) {
	zr, err := zip.NewReader(bytes.NewReader(data), int64(len(data)))
	if err != nil {
		return "", false
	}
	var docXML []byte
	for _, f := range zr.File {
		if f.Name != "word/document.xml" {
			continue
		}
		rc, err := f.Open()
		if err != nil {
			return "", false
		}
		docXML, err = io.ReadAll(rc)
		rc.Close()
		if err != nil {
			return "", false
		}
		break
	}
	if docXML == nil {
		return "", false
	}
	text := strings.TrimSpace(docxXMLToText(string(docXML)))
	if text == "" {
		return "", false
	}
	return text, true
}

func docxXMLToText(x string) string {
	x = strings.ReplaceAll(x, "</w:p>", "\n")
	x = strings.ReplaceAll(x, "<w:tab/>", "\t")
	x = strings.ReplaceAll(x, "<w:br/>", "\n")
	var b strings.Builder
	inTag := false
	for _, r := range x {
		switch r {
		case '<':
			inTag = true
		case '>':
			inTag = false
		default:
			if !inTag {
				b.WriteRune(r)
			}
		}
	}
	return html.UnescapeString(b.String())
}

// imageParts returns base64-encoded image parts for the given attachment IDs,
// so vision-capable models can actually see the uploaded images. Non-image
// attachments and images over maxImageBytes are skipped.
func (s *Server) imageParts(ids []string) []*pb.MultiModalPart {
	var parts []*pb.MultiModalPart
	for _, id := range ids {
		meta, data, err := s.loadAttachment(id)
		if err != nil {
			continue
		}
		if !isImageContent(meta.ContentType) || len(data) > maxImageBytes {
			continue
		}
		contentType := meta.ContentType
		if contentType == "" {
			contentType = http.DetectContentType(data)
		}
		parts = append(parts, &pb.MultiModalPart{
			ContentType: contentType,
			Data:        base64.StdEncoding.EncodeToString(data),
		})
	}
	return parts
}

func isImageContent(contentType string) bool {
	return strings.HasPrefix(contentType, "image/")
}

func isTextContent(contentType string) bool {
	return strings.HasPrefix(contentType, "text/") ||
		strings.Contains(contentType, "json") ||
		strings.Contains(contentType, "xml") ||
		strings.Contains(contentType, "javascript") ||
		strings.Contains(contentType, "csv") ||
		strings.Contains(contentType, "yaml") ||
		strings.Contains(contentType, "markdown")
}

func looksText(data []byte) bool {
	if !utf8.Valid(data) {
		return false
	}
	for _, b := range data {
		if b == 0 {
			return false
		}
	}
	return true
}

// cancelChat stops an in-flight chat request for the given session. It is a
// no-op when no request is currently running.
func (s *Server) cancelChat(c *gin.Context) {
	id := c.Param("id")
	if v, ok := s.activeChats.Load(id); ok {
		if cancel, ok := v.(context.CancelFunc); ok {
			log.Printf("chat cancel requested session=%s", id)
			cancel()
		}
	}
	c.JSON(http.StatusOK, gin.H{"cancelled": true})
}

func (s *Server) listWorkspaces(c *gin.Context) {
	workspaces, err := s.workspaces.List()
	if err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	c.JSON(http.StatusOK, gin.H{"workspaces": workspaces})
}

type createWorkspaceRequest struct {
	Name  string   `json:"name"`
	Path  string   `json:"path"`
	Tools []string `json:"tools"`
}

func (s *Server) createWorkspace(c *gin.Context) {
	var req createWorkspaceRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}
	path := strings.TrimSpace(req.Path)
	if path == "" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "path is required"})
		return
	}
	path = filepath.Clean(path)
	info, err := os.Stat(path)
	if err != nil || !info.IsDir() {
		c.JSON(http.StatusBadRequest, gin.H{"error": "workspace path must be an existing directory"})
		return
	}
	name := strings.TrimSpace(req.Name)
	if name == "" {
		name = filepath.Base(path)
	}
	ws := &workspace.Workspace{
		ID:    uuid.New().String(),
		Name:  name,
		Path:  path,
		Tools: req.Tools,
	}
	if err := s.workspaces.Create(ws); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	log.Printf("workspace created id=%s name=%q path=%q tools=%v", ws.ID, ws.Name, ws.Path, ws.Tools)
	c.JSON(http.StatusCreated, ws)
}

func (s *Server) deleteWorkspace(c *gin.Context) {
	id := c.Param("id")
	if err := s.workspaces.Delete(id); err != nil {
		c.JSON(http.StatusInternalServerError, gin.H{"error": err.Error()})
		return
	}
	log.Printf("workspace deleted id=%s", id)
	c.JSON(http.StatusOK, gin.H{"deleted": true})
}

// downloadWorkspaceFile serves files written by the write_file tool into a
// workspace's outputs directory. The file name is restricted to a simple base
// name to prevent directory traversal.
func (s *Server) downloadWorkspaceFile(c *gin.Context) {
	ws, err := s.workspaces.Get(c.Param("id"))
	if err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "workspace not found"})
		return
	}
	name := c.Param("name")
	name = filepath.Base(name)
	if decoded, err := url.PathUnescape(name); err == nil {
		name = decoded
	}
	if name == "" || name == "." || name == "/" {
		c.JSON(http.StatusBadRequest, gin.H{"error": "invalid file name"})
		return
	}
	path := filepath.Join(ws.Path, "outputs", name)
	data, err := os.ReadFile(path)
	if err != nil {
		c.JSON(http.StatusNotFound, gin.H{"error": "file not found"})
		return
	}
	contentType := http.DetectContentType(data)
	c.Header("Content-Disposition", "attachment; filename=\""+name+"\"")
	c.Data(http.StatusOK, contentType, data)
}

func contextWithTimeout(seconds ...int) (context.Context, context.CancelFunc) {
	d := 30
	if len(seconds) > 0 && seconds[0] > 0 {
		d = seconds[0]
	}
	return context.WithTimeout(context.Background(), time.Duration(d)*time.Second)
}
