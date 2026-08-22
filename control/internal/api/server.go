package api

import (
	"archive/zip"
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"html"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/daheige/rsmgo/control/internal/engine"
	"github.com/daheige/rsmgo/control/internal/session"
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
	router          *gin.Engine
	providers       []string
	defaultProvider string
	uploadDir       string
}

func NewServer(engineClient *engine.Client, store *session.Store, providers []string, uploadDir string) *Server {
	gin.SetMode(gin.ReleaseMode)
	r := gin.New()
	r.Use(gin.Recovery())
	r.Use(corsMiddleware())

	defaultProvider := "openai"
	if len(providers) > 0 {
		defaultProvider = providers[0]
	}

	_ = os.MkdirAll(uploadDir, 0o755)

	s := &Server{
		engine:          engineClient,
		sessions:        store,
		router:          r,
		providers:       providers,
		defaultProvider: defaultProvider,
		uploadDir:       uploadDir,
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
	s.router.DELETE("/api/v1/sessions/:id", s.deleteSession)
	s.router.POST("/api/v1/uploads", s.uploadFile)
	s.router.GET("/api/v1/uploads/:id", s.downloadFile)
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

type updateSessionRequest struct {
	Title  *string `json:"title"`
	Pinned *bool   `json:"pinned"`
}

func (s *Server) updateSession(c *gin.Context) {
	id := c.Param("id")
	var req updateSessionRequest
	if err := c.ShouldBindJSON(&req); err != nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": err.Error()})
		return
	}
	if req.Title == nil && req.Pinned == nil {
		c.JSON(http.StatusBadRequest, gin.H{"error": "nothing to update"})
		return
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

	sess.Messages = append(sess.Messages, session.Message{
		Role:    "user",
		Content: content,
		SentAt:  time.Now().UTC(),
	})

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

	ctx, cancel := contextWithTimeout(120)
	defer cancel()
	resp, err := s.engine.Chat(ctx, &pb.ChatRequest{
		SessionId: id,
		Messages:  pbMessages,
		Provider:  sess.Provider,
		Model:     sess.Model,
		ToolNames: toolNames,
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

func contextWithTimeout(seconds ...int) (context.Context, context.CancelFunc) {
	d := 30
	if len(seconds) > 0 && seconds[0] > 0 {
		d = seconds[0]
	}
	return context.WithTimeout(context.Background(), time.Duration(d)*time.Second)
}
