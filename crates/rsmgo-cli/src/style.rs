//! ANSI style helpers: colors only when stdout is a terminal.
use std::io::IsTerminal;

pub fn styled(s: &str, code: &str) -> String {
    if std::io::stdout().is_terminal() {
        format!("\x1b[{}m{}\x1b[0m", code, s)
    } else {
        s.to_string()
    }
}

pub fn dim(s: &str) -> String {
    styled(s, "2;3")
}

pub fn cyan(s: &str) -> String {
    styled(s, "36;1")
}
