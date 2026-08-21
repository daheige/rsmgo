package session

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"sync"
	"time"
)

type Session struct {
	ID        string    `json:"id"`
	Title     string    `json:"title"`
	Provider  string    `json:"provider"`
	Model     string    `json:"model"`
	Messages  []Message `json:"messages"`
	Pinned    bool      `json:"pinned"`
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

type Message struct {
	Role    string    `json:"role"`
	Content string    `json:"content"`
	SentAt  time.Time `json:"sent_at"`
}

type Store struct {
	mu  sync.RWMutex
	dir string
}

func NewStore(dir string) *Store {
	_ = os.MkdirAll(dir, 0o755)
	return &Store{dir: dir}
}

func (s *Store) path(id string) string {
	return filepath.Join(s.dir, fmt.Sprintf("%s.json", id))
}

func (s *Store) Create(sess *Session) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	sess.CreatedAt = time.Now().UTC()
	sess.UpdatedAt = sess.CreatedAt
	if sess.Messages == nil {
		sess.Messages = []Message{}
	}
	data, err := json.MarshalIndent(sess, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(s.path(sess.ID), data, 0o644)
}

func (s *Store) Get(id string) (*Session, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.read(id)
}

// read loads a session without taking a lock. Callers must hold s.mu.
func (s *Store) read(id string) (*Session, error) {
	data, err := os.ReadFile(s.path(id))
	if err != nil {
		return nil, err
	}
	var sess Session
	if err := json.Unmarshal(data, &sess); err != nil {
		return nil, err
	}
	if sess.Messages == nil {
		sess.Messages = []Message{}
	}
	return &sess, nil
}

func (s *Store) Update(sess *Session) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	sess.UpdatedAt = time.Now().UTC()
	if sess.Messages == nil {
		sess.Messages = []Message{}
	}
	data, err := json.MarshalIndent(sess, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(s.path(sess.ID), data, 0o644)
}

func (s *Store) Delete(id string) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return os.Remove(s.path(id))
}

// Patch loads a session, applies the mutation, and persists the result.
func (s *Store) Patch(id string, mutate func(*Session) error) (*Session, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	sess, err := s.read(id)
	if err != nil {
		return nil, err
	}
	if err := mutate(sess); err != nil {
		return nil, err
	}
	sess.UpdatedAt = time.Now().UTC()
	data, err := json.MarshalIndent(sess, "", "  ")
	if err != nil {
		return nil, err
	}
	if err := os.WriteFile(s.path(sess.ID), data, 0o644); err != nil {
		return nil, err
	}
	return sess, nil
}

func (s *Store) List() ([]*Session, error) {
	s.mu.RLock()
	defer s.mu.RUnlock()
	entries, err := os.ReadDir(s.dir)
	if err != nil {
		return nil, err
	}
	sessions := make([]*Session, 0)
	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".json" {
			continue
		}
		data, err := os.ReadFile(filepath.Join(s.dir, entry.Name()))
		if err != nil {
			continue
		}
		var sess Session
		if err := json.Unmarshal(data, &sess); err != nil {
			continue
		}
		sessions = append(sessions, &sess)
	}
	sort.SliceStable(sessions, func(i, j int) bool {
		if sessions[i].Pinned != sessions[j].Pinned {
			return sessions[i].Pinned
		}
		return sessions[i].UpdatedAt.After(sessions[j].UpdatedAt)
	})
	return sessions, nil
}
