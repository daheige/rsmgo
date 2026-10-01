//! Lightweight terminal syntax highlighting for fenced code blocks.
//!
//! termimad has no built-in highlighter, so code is tokenized here with a
//! small line-oriented scanner. It is deliberately simple — enough to make
//! comments, strings, keywords and numbers legible — without pulling in a
//! full highlighter such as syntect. Multi-line state (block comments and
//! triple-quoted strings) is tracked across lines.
//!
//! Language behavior (comment syntax, keyword set, case sensitivity) comes
//! from the table in [`keywords`]; see its docs for coverage.

mod keywords;

use keywords::{lang_spec, CommentStyle, LangSpec};

const RESET: &str = "\x1b[0m";

fn styled(code: &str, text: &str) -> String {
    format!("\x1b[{}m{}{}", code, text, RESET)
}

/// Whether `word` is in the spec's keyword set, honoring `ignore_case`.
fn is_keyword(spec: &LangSpec, word: &str) -> bool {
    if spec.ignore_case {
        spec.keywords.contains(&word.to_ascii_lowercase().as_str())
    } else {
        spec.keywords.contains(&word)
    }
}

/// Whether `word` is a boolean / nil literal, colored like a number.
fn is_literal(word: &str) -> bool {
    matches!(
        word,
        "true" | "false" | "null" | "none" | "None" | "NONE" | "nil" | "undefined" | "True"
            | "False" | "TRUE" | "FALSE" | "NULL" | "Ok" | "Error" | "Some" | "Just" | "Nothing"
            | "Inf" | "NaN" | "NA"
    )
}

/// Byte offset of `needle` starting at or after `from`, if present.
fn find_subslice(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    hay.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// Byte offset of `needle` starting at or after `from`, if present.
fn find_str(hay: &str, from: usize, needle: &str) -> Option<usize> {
    hay.get(from..)?.find(needle).map(|p| from + p)
}

/// Byte offset just past the closing quote of a string opened at `start`
/// (which points at the opening quote). Handles backslash escapes. Returns
/// `None` when the string runs off the end of the line.
fn string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let quote = bytes[start];
    let n = bytes.len();
    let mut i = start + 1;
    while i < n {
        let b = bytes[i];
        if b == b'\\' {
            i += 2;
            continue;
        }
        if b == quote {
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

/// Color one line of code into `out`, advancing `state` across line boundaries.
///
/// All indices are byte offsets, but are only ever advanced by whole ASCII
/// characters or `char::len_utf8()` for non-ASCII text, so every slice lands
/// on a UTF-8 boundary.
fn highlight_line(spec: &LangSpec, state: &mut SpanState, line: &str, out: &mut String) {
    let bytes = line.as_bytes();
    let n = bytes.len();
    let mut i = 0;

    // Continue a multi-line block comment or triple-quoted string.
    if state.block_comment {
        match find_subslice(bytes, i, b"*/") {
            Some(end) => {
                out.push_str(&styled("2;3", &line[..end + 2]));
                i = end + 2;
            }
            None => {
                out.push_str(&styled("2;3", line));
                return;
            }
        }
        state.block_comment = false;
    }
    if let Some(triple) = state.triple.take() {
        match find_str(line, i, &triple) {
            Some(end) => {
                out.push_str(&styled("32", &line[..end + triple.len()]));
                i = end + triple.len();
            }
            None => {
                out.push_str(&styled("32", line));
                state.triple = Some(triple);
                return;
            }
        }
    }

    while i < n {
        let c = bytes[i];

        // Line / block comments.
        match spec.comments {
            CommentStyle::None => {}
            CommentStyle::CStyle if c == b'/' && i + 1 < n && bytes[i + 1] == b'/' => {
                out.push_str(&styled("2;3", &line[i..]));
                return;
            }
            CommentStyle::CStyle if c == b'/' && i + 1 < n && bytes[i + 1] == b'*' => {
                match find_subslice(bytes, i + 2, b"*/") {
                    Some(end) => {
                        out.push_str(&styled("2;3", &line[i..end + 2]));
                        i = end + 2;
                        continue;
                    }
                    None => {
                        out.push_str(&styled("2;3", &line[i..]));
                        state.block_comment = true;
                        return;
                    }
                }
            }
            CommentStyle::Hash if c == b'#' => {
                out.push_str(&styled("2;3", &line[i..]));
                return;
            }
            CommentStyle::Dash if c == b'-' && i + 1 < n && bytes[i + 1] == b'-' => {
                out.push_str(&styled("2;3", &line[i..]));
                return;
            }
            CommentStyle::Percent if c == b'%' => {
                out.push_str(&styled("2;3", &line[i..]));
                return;
            }
            CommentStyle::Semicolon if c == b';' => {
                out.push_str(&styled("2;3", &line[i..]));
                return;
            }
            CommentStyle::Bang if c == b'!' => {
                out.push_str(&styled("2;3", &line[i..]));
                return;
            }
            CommentStyle::Apostrophe if c == b'\'' => {
                out.push_str(&styled("2;3", &line[i..]));
                return;
            }
            CommentStyle::ParenStar if c == b'(' && i + 1 < n && bytes[i + 1] == b'*' => {
                match find_subslice(bytes, i + 2, b"*)") {
                    Some(end) => {
                        out.push_str(&styled("2;3", &line[i..end + 2]));
                        i = end + 2;
                        continue;
                    }
                    None => {
                        out.push_str(&styled("2;3", &line[i..]));
                        state.block_comment = true;
                        return;
                    }
                }
            }
            _ => {}
        }

        // Triple-quoted strings (Python docstrings / multi-line strings).
        if i + 2 < n && (bytes[i] == b'"' || bytes[i] == b'\'') && bytes[i] == bytes[i + 1]
            && bytes[i] == bytes[i + 2]
        {
            let delim = &line[i..i + 3];
            match find_str(line, i + 3, delim) {
                Some(end) => {
                    out.push_str(&styled("32", &line[i..end + 3]));
                    i = end + 3;
                    continue;
                }
                None => {
                    out.push_str(&styled("32", &line[i..]));
                    state.triple = Some(delim.to_string());
                    return;
                }
            }
        }

        // Single/double/backtick-quoted strings.
        if c == b'"' || c == b'\'' || c == b'`' {
            match string_end(bytes, i) {
                Some(end) => {
                    out.push_str(&styled("32", &line[i..end]));
                    i = end;
                    continue;
                }
                None => {
                    out.push_str(&styled("32", &line[i..]));
                    return;
                }
            }
        }

        // Numbers.
        if c.is_ascii_digit() {
            let mut j = i + 1;
            while j < n
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'.' || bytes[j] == b'_')
            {
                j += 1;
            }
            out.push_str(&styled("33", &line[i..j]));
            i = j;
            continue;
        }

        // Identifiers and keywords.
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i + 1;
            while j < n && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let word = &line[i..j];
            if is_keyword(spec, word) {
                out.push_str(&styled("1;36", word));
            } else if is_literal(word) {
                out.push_str(&styled("33", word));
            } else {
                out.push_str(word);
            }
            i = j;
            continue;
        }

        // Anything else (punctuation, non-ASCII): copy one char verbatim.
        let ch = line[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
}

/// State carried across lines: a block comment (`/* ... */` or `(* ... *)`)
/// or a triple-quoted string (`"""` / `'''`) can span more than one line.
#[derive(Default)]
struct SpanState {
    block_comment: bool,
    triple: Option<String>,
}

/// Incremental syntax highlighter for one fenced code block.
pub struct CodeHighlighter {
    lang: String,
    spec: &'static LangSpec,
    state: SpanState,
}

impl CodeHighlighter {
    pub fn new() -> Self {
        Self {
            lang: String::new(),
            spec: &keywords::GENERIC,
            state: SpanState::default(),
        }
    }

    /// Clear multi-line state when a code block ends.
    pub fn reset(&mut self) {
        self.state = SpanState::default();
    }

    /// Highlight one line of a code block, returning ANSI-colored text.
    pub fn highlight_line(&mut self, lang: &str, line: &str) -> String {
        // The language tag can change between blocks; only re-resolve when it
        // does (termimad may reuse the renderer across fenced blocks).
        if self.lang != lang {
            self.lang = lang.to_string();
            self.spec = lang_spec(lang);
            self.reset();
        }
        let mut out = String::with_capacity(line.len() + 16);
        highlight_line(self.spec, &mut self.state, line, &mut out);
        out
    }
}

impl Default for CodeHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn highlight_all(lang: &str, code: &str) -> String {
        let mut hl = CodeHighlighter::new();
        let mut out = String::new();
        for line in code.lines() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&hl.highlight_line(lang, line));
        }
        out
    }

    #[test]
    fn colors_js_keywords_strings_comments() {
        let out = highlight_all("js", "const x = 'hi'; // note");
        assert!(out.contains("\x1b[1;36mconst\x1b[0m"), "keyword: {out}");
        assert!(out.contains("\x1b[32m'hi'\x1b[0m"), "string: {out}");
        assert!(out.contains("\x1b[2;3m// note\x1b[0m"), "comment: {out}");
        assert!(!out.contains("note\x1b[0m;"), "comment swallows only the tail");
    }

    #[test]
    fn colors_python_hash_comment_and_number() {
        let out = highlight_all("python", "n = 42  # answer");
        assert!(out.contains("\x1b[33m42\x1b[0m"), "number: {out}");
        assert!(out.contains("\x1b[2;3m# answer\x1b[0m"), "comment: {out}");
    }

    #[test]
    fn tracks_block_comment_across_lines() {
        let out = highlight_all("js", "/* open\nstill */x = 1");
        assert!(out.contains("\x1b[2;3m/* open\x1b[0m"), "first line: {out}");
        assert!(out.contains("\x1b[2;3mstill */\x1b[0m"), "second line: {out}");
    }

    #[test]
    fn colors_rust_keywords() {
        let out = highlight_all("rust", "pub fn main() { let mut x = 1; }");
        for kw in ["pub", "fn", "let", "mut"] {
            assert!(
                out.contains(&format!("\x1b[1;36m{kw}\x1b[0m")),
                "rust keyword {kw}: {out}"
            );
        }
    }

    #[test]
    fn colors_go_comments_and_keywords() {
        let out = highlight_all("go", "package main // entry");
        assert!(out.contains("\x1b[1;36mpackage\x1b[0m"), "go keyword: {out}");
        assert!(out.contains("\x1b[2;3m// entry\x1b[0m"), "go comment: {out}");
    }

    #[test]
    fn colors_java_keywords() {
        let out = highlight_all("java", "public class Foo { static void main() {} }");
        for kw in ["public", "class", "static", "void"] {
            assert!(
                out.contains(&format!("\x1b[1;36m{kw}\x1b[0m")),
                "java keyword {kw}: {out}"
            );
        }
    }

    #[test]
    fn colors_json_without_comments() {
        let out = highlight_all("json", r#"{ "key": true, "n": 3 }"#);
        assert!(out.contains("\x1b[32m\"key\"\x1b[0m"), "json key string: {out}");
        assert!(out.contains("\x1b[33mtrue\x1b[0m"), "json literal: {out}");
        assert!(out.contains("\x1b[33m3\x1b[0m"), "json number: {out}");
    }

    #[test]
    fn sql_keywords_match_case_insensitively() {
        let out = highlight_all("sql", "SELECT name FROM users -- list");
        assert!(out.contains("\x1b[1;36mSELECT\x1b[0m"), "uppercase: {out}");
        assert!(out.contains("\x1b[1;36mFROM\x1b[0m"), "uppercase from: {out}");
        assert!(out.contains("\x1b[2;3m-- list\x1b[0m"), "comment: {out}");
        let lower = highlight_all("sql", "select id from t where x = 1");
        for kw in ["select", "from", "where"] {
            assert!(
                lower.contains(&format!("\x1b[1;36m{kw}\x1b[0m")),
                "lowercase {kw}: {lower}"
            );
        }
    }

    #[test]
    fn c_plus_plus_alias_resolves_to_c_spec() {
        let out = highlight_all("c++", "int main() { return 0; } // ok");
        assert!(out.contains("\x1b[1;36mreturn\x1b[0m"), "c++ keyword: {out}");
        assert!(out.contains("\x1b[2;3m// ok\x1b[0m"), "c++ comment: {out}");
    }

    #[test]
    fn shell_comments_and_keywords() {
        let out = highlight_all("bash", "#!/bin/bash\nfor f in *; do echo $f; done");
        assert!(out.contains("\x1b[2;3m#!/bin/bash\x1b[0m"), "shebang: {out}");
        assert!(out.contains("\x1b[1;36mfor\x1b[0m"), "for: {out}");
        assert!(out.contains("\x1b[1;36mdo\x1b[0m"), "do: {out}");
    }

    #[test]
    fn rust_is_not_hash_comment_style() {
        // Regression: the old substring match let "r" claim Rust for the
        // hash-comment family. Rust uses C-style comments.
        let out = highlight_all("rust", "let x = 1; // tail");
        assert!(out.contains("\x1b[2;3m// tail\x1b[0m"), "rust // comment: {out}");
    }
}
