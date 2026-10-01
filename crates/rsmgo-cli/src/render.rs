//! Incremental Markdown rendering for the terminal.
//!
//! Streams the model's answer through a renderer so `**bold**`, `## headers`
//! and `| tables |` arrive as styled terminal output instead of raw markup.
//!
//! The renderer is line-oriented: a line is rendered as soon as it is
//! complete, tables are accumulated and rendered as one block, and fenced
//! code is passed through untouched. When stdout is not a terminal (piped or
//! redirected) the raw text is written unchanged so scripts keep clean input.

use crate::highlight::CodeHighlighter;
use std::io::{self, IsTerminal, Write};
use termimad::crossterm::style::Attribute;
use termimad::MadSkin;

pub struct MarkdownRenderer {
    enabled: bool,
    skin: MadSkin,
    /// Text that has not seen a newline yet.
    buf: String,
    /// Pending consecutive table lines, rendered as one block.
    table: Vec<String>,
    /// Whether we are inside a fenced code block.
    in_code: bool,
    /// Language declared on the current code fence (```` ```js ````).
    code_lang: Option<String>,
    highlighter: CodeHighlighter,
}

impl MarkdownRenderer {
    pub fn new() -> Self {
        // termimad's default skin underlines every header; drop the underline
        // and keep headings bold so they stand out without the extra line.
        let mut skin = MadSkin::default();
        for header in &mut skin.headers {
            header.compound_style.remove_attr(Attribute::Underlined);
            header.compound_style.add_attr(Attribute::Bold);
        }

        Self {
            enabled: io::stdout().is_terminal(),
            skin,
            buf: String::new(),
            table: Vec::new(),
            in_code: false,
            code_lang: None,
            highlighter: CodeHighlighter::new(),
        }
    }

    /// Feed a streamed text fragment; completed lines are rendered and
    /// printed immediately.
    pub fn feed(&mut self, text: &str) {
        if !self.enabled {
            print!("{}", text);
            let _ = io::stdout().flush();
            return;
        }
        self.buf.push_str(text);
        while let Some(pos) = self.buf.find('\n') {
            // Drain through the newline; `pos` is a byte offset of an ASCII
            // char, so it is always on a char boundary even with multi-byte
            // text in earlier lines.
            let line: String = self.buf[..pos].to_string();
            self.buf.drain(..=pos);
            self.handle_line(&line);
        }
        let _ = io::stdout().flush();
    }

    /// Render whatever is left at the end of the stream (a final line
    /// without a trailing newline).
    pub fn flush(&mut self) {
        if !self.enabled {
            return;
        }
        self.flush_table();
        if !self.buf.is_empty() {
            let line = std::mem::take(&mut self.buf);
            self.render_markdown(&line);
        }
        let _ = io::stdout().flush();
    }

    fn handle_line(&mut self, line: &str) {
        let trimmed = line.trim_start();
        if let Some(fence) = trimmed.strip_prefix("```") {
            self.flush_table();
            if self.in_code {
                self.in_code = false;
                self.code_lang = None;
                self.highlighter.reset();
            } else {
                self.in_code = true;
                // `` ```js `` / `` ```rust `` → language; bare fences get the
                // generic C-style fallback.
                let lang = fence.trim();
                self.code_lang = if lang.is_empty() {
                    None
                } else {
                    Some(lang.to_string())
                };
                self.highlighter.reset();
            }
            // Fences are structural: print them dimmed, code between fences
            // is syntax-highlighted.
            println!("{}", dim(line));
            return;
        }
        if self.in_code {
            // Syntax-highlight while preserving code indentation exactly.
            let lang = self.code_lang.as_deref().unwrap_or("");
            let highlighted = self.highlighter.highlight_line(lang, line);
            println!("{}", highlighted);
            return;
        }
        if trimmed.starts_with('|') {
            self.table.push(line.to_string());
            return;
        }
        self.flush_table();
        self.render_markdown(line);
    }

    fn flush_table(&mut self) {
        if self.table.is_empty() {
            return;
        }
        let block = self.table.join("\n");
        self.table.clear();
        self.skin.print_text(&block);
    }

    fn render_markdown(&self, line: &str) {
        // Skip blank lines quietly: print_text would add vertical padding.
        if line.trim().is_empty() {
            println!();
            return;
        }
        self.skin.print_text(line);
    }
}

fn dim(s: &str) -> String {
    format!("\x1b[2m{}\x1b[0m", s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_are_bold_not_underlined() {
        let renderer = MarkdownRenderer::new();
        assert!(!renderer.skin.headers.is_empty(), "skin has header styles");
        for header in &renderer.skin.headers {
            assert!(
                !header.compound_style.has_attr(Attribute::Underlined),
                "header should not be underlined"
            );
            assert!(
                header.compound_style.has_attr(Attribute::Bold),
                "header should be bold"
            );
        }

        // End-to-end: rendering a header must not emit the underline SGR code.
        let rendered = format!("{}", renderer.skin.text("# Title", Some(80)));
        assert!(
            !rendered.contains("\x1b[4m"),
            "rendered header must not be underlined: {rendered:?}"
        );
        assert!(
            rendered.contains("\x1b[1m"),
            "rendered header should be bold: {rendered:?}"
        );
    }
}
