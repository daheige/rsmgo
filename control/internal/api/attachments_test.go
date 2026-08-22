package api

import (
	"archive/zip"
	"bytes"
	"testing"
)

func TestTruncateTextDoesNotSplitRune(t *testing.T) {
	s := "你好，世界！This is a test"
	out := truncateText(s, 8)
	// The first 6 bytes are the two runes "你好"; the boundary at 8 lands in the
	// middle of "，" so the cut must step back to 6.
	if want := s[:6] + "\n...(truncated)"; out != want {
		t.Fatalf("unexpected truncation: got %q want %q", out, want)
	}
}

func TestExtractDocxText(t *testing.T) {
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	w, err := zw.Create("word/document.xml")
	if err != nil {
		t.Fatal(err)
	}
	_, err = w.Write([]byte(`<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:r><w:t>World &amp; more</w:t></w:r></w:p>`))
	if err != nil {
		t.Fatal(err)
	}
	if err := zw.Close(); err != nil {
		t.Fatal(err)
	}

	text, ok := extractDocxText(buf.Bytes())
	if !ok {
		t.Fatal("expected DOCX extraction to succeed")
	}
	if want := "Hello\nWorld & more"; text != want {
		t.Fatalf("unexpected text: got %q want %q", text, want)
	}
}

func TestExtractDocxTextRejectsNonDocxZip(t *testing.T) {
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	w, _ := zw.Create("some/other.xml")
	w.Write([]byte("<x/>"))
	zw.Close()

	if _, ok := extractDocxText(buf.Bytes()); ok {
		t.Fatal("expected non-DOCX zip to be rejected")
	}
}
