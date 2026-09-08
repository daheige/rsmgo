//! SQLite database query tool. Opens a database file read-only by default and
//! returns matching rows as JSON; explicit opt-in (`write: true`) is required
//! for statements that modify data.

use crate::error::{Result, RsmgoError};
use crate::tools::{Tool, ToolContext};
use async_trait::async_trait;
use rusqlite::{OpenFlags, Row};
use serde_json::json;
use std::path::PathBuf;

/// Cap the number of rows returned to the model per query.
const MAX_ROWS: usize = 500;

/// Cap the total serialized output, in characters.
const MAX_OUTPUT_CHARS: usize = 12000;

/// Extract the file path from a `sqlite://path` (or plain path) connection
/// string. Relative paths resolve against the active workspace.
fn resolve_db_path(url: &str, ctx: &ToolContext) -> Result<PathBuf> {
    let path = url
        .strip_prefix("sqlite://")
        .or_else(|| url.strip_prefix("sqlite:"))
        .unwrap_or(url);
    if path.is_empty() {
        return Err(RsmgoError::Tool(
            "empty database path; expected 'sqlite://path/to.db'".to_string(),
        ));
    }
    Ok(ctx.resolve(path))
}

/// Split SQL into top-level statements, honoring single-quoted strings,
/// double-quoted identifiers, and `--` / `*​/` comments, so a semicolon inside
/// a string literal does not count as a statement separator.
fn split_statements(sql: &str) -> Vec<String> {
    #[derive(PartialEq)]
    enum State {
        Normal,
        SingleQuote,
        DoubleQuote,
        LineComment,
        BlockComment,
    }
    let mut state = State::Normal;
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match state {
            State::Normal => match c {
                '\'' => {
                    state = State::SingleQuote;
                    current.push(c);
                }
                '"' => {
                    state = State::DoubleQuote;
                    current.push(c);
                }
                '-' if chars.peek() == Some(&'-') => {
                    chars.next();
                    state = State::LineComment;
                }
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    state = State::BlockComment;
                }
                ';' => {
                    if !current.trim().is_empty() {
                        statements.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(c),
            },
            State::SingleQuote => {
                current.push(c);
                if c == '\'' {
                    // SQL escapes a quote inside a string by doubling it.
                    if chars.peek() == Some(&'\'') {
                        current.push(chars.next().unwrap());
                    } else {
                        state = State::Normal;
                    }
                }
            }
            State::DoubleQuote => {
                current.push(c);
                if c == '"' {
                    state = State::Normal;
                }
            }
            State::LineComment => {
                if c == '\n' {
                    state = State::Normal;
                }
            }
            State::BlockComment => {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    state = State::Normal;
                }
            }
        }
    }
    if !current.trim().is_empty() {
        statements.push(current);
    }
    statements
}

/// First-keyword check for data-modifying statements. This runs before the
/// connection is opened so the model gets a friendly error even when the
/// database file does not exist yet; `Statement::readonly()` remains the
/// authoritative gate afterwards (it catches CTE/PRAGMA tricks).
fn is_write_statement(sql: &str) -> bool {
    let head = sql
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_uppercase();
    matches!(
        head.as_str(),
        "INSERT"
            | "UPDATE"
            | "DELETE"
            | "REPLACE"
            | "DROP"
            | "ALTER"
            | "CREATE"
            | "VACUUM"
            | "REINDEX"
            | "TRUNCATE"
            | "ATTACH"
            | "DETACH"
    )
}

pub struct DbQueryTool;

#[async_trait]
impl Tool for DbQueryTool {
    fn name(&self) -> &str {
        "db_query"
    }

    fn description(&self) -> &str {
        "Run a single SQL statement against a SQLite database. Read-only by default; pass write: true to allow INSERT/UPDATE/DELETE/DDL. Results are returned as JSON rows (max 500 rows). Connection string: 'sqlite://path/to.db' (relative paths resolve against the workspace)."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Database connection string, e.g. 'sqlite://data/app.db'" },
                "sql": { "type": "string", "description": "A single SQL statement" },
                "write": { "type": "boolean", "description": "Allow data-modifying statements (default false)" }
            },
            "required": ["url", "sql"]
        })
    }

    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> Result<String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'url' argument".to_string()))?;
        let sql = args["sql"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'sql' argument".to_string()))?;
        let write = args["write"].as_bool().unwrap_or(false);

        let statements = split_statements(sql);
        if statements.len() > 1 {
            return Err(RsmgoError::Tool(
                "multiple SQL statements are not allowed; send one statement per call".to_string(),
            ));
        }
        if statements.is_empty() {
            return Err(RsmgoError::Tool("empty SQL statement".to_string()));
        }
        let sql = statements[0].trim();
        if is_write_statement(sql) && !write {
            return Err(RsmgoError::Tool(
                "this statement modifies data; re-run with write: true to confirm".to_string(),
            ));
        }

        let path = resolve_db_path(url, ctx)?;
        let flags = if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let conn = rusqlite::Connection::open_with_flags(&path, flags).map_err(|e| {
            RsmgoError::Tool(format!(
                "failed to open database '{}': {}",
                path.display(),
                e
            ))
        })?;

        run_statement(&conn, sql, write)
    }
}

fn run_statement(conn: &rusqlite::Connection, sql: &str, write: bool) -> Result<String> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| RsmgoError::Tool(format!("SQL error: {}", e)))?;

    // SQLite's own verdict on whether this statement changes data. This is
    // the authoritative read-only gate (it catches CTEs and PRAGMA tricks
    // that a keyword check would miss).
    if !stmt.readonly() && !write {
        return Err(RsmgoError::Tool(
            "this statement modifies data; re-run with write: true to confirm".to_string(),
        ));
    }

    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    // No result columns → a pure modification statement; report affected rows.
    if column_names.is_empty() {
        let affected = stmt
            .execute([])
            .map_err(|e| RsmgoError::Tool(format!("SQL error: {}", e)))?;
        return Ok(format!("OK, {} row(s) affected", affected));
    }

    let rows = stmt
        .query_map([], |row| row_to_json(row, &column_names))
        .map_err(|e| RsmgoError::Tool(format!("SQL error: {}", e)))?;

    let mut values = Vec::new();
    for row in rows {
        values.push(row?);
        if values.len() >= MAX_ROWS {
            break;
        }
    }

    let truncated = values.len() >= MAX_ROWS;
    let mut out = format!(
        "{} row(s){}\n",
        values.len(),
        if truncated { " (truncated at 500)" } else { "" }
    );
    let json = serde_json::to_string_pretty(&values)
        .map_err(|e| RsmgoError::Tool(format!("failed to serialize rows: {}", e)))?;
    let json: String = json.chars().take(MAX_OUTPUT_CHARS).collect();
    out.push_str(&json);
    Ok(out)
}

/// Convert one result row into a JSON object keyed by column name.
fn row_to_json(row: &Row, columns: &[String]) -> rusqlite::Result<serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (i, name) in columns.iter().enumerate() {
        let value: rusqlite::types::Value = row.get(i)?;
        let value = match value {
            rusqlite::types::Value::Null => serde_json::Value::Null,
            rusqlite::types::Value::Integer(n) => serde_json::json!(n),
            rusqlite::types::Value::Real(f) => serde_json::json!(f),
            rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
            rusqlite::types::Value::Blob(b) => {
                serde_json::json!(format!("{} bytes blob (content omitted)", b.len()))
            }
        };
        map.insert(name.clone(), value);
    }
    Ok(serde_json::Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_db(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rsmgo-db-test-{}-{}", name, nanos));
        fs::create_dir_all(&dir).unwrap();
        dir.join("test.db")
    }

    #[test]
    fn split_statements_respects_strings_and_comments() {
        assert_eq!(split_statements("SELECT 1").len(), 1);
        assert_eq!(split_statements("SELECT 1;").len(), 1);
        assert_eq!(split_statements("SELECT 1; SELECT 2").len(), 2);
        assert_eq!(
            split_statements("SELECT ';' AS semi; -- trailing; comment\nSELECT 2").len(),
            2
        );
        assert_eq!(split_statements("SELECT 1 /* ; */ + 2").len(), 1);
        assert_eq!(split_statements("  -- only a comment\n ").len(), 0);
        assert_eq!(
            split_statements("INSERT INTO t VALUES ('it''s; ok')").len(),
            1
        );
    }

    #[tokio::test]
    async fn db_query_roundtrip_read_and_write() {
        let db = temp_db("roundtrip");
        let tool = DbQueryTool;
        let ctx = ToolContext::default();
        let url = format!("sqlite://{}", db.display());

        // Create + insert require write: true.
        let err = tool
            .execute(
                json!({"url": url, "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)"}),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("write: true"), "{}", err);

        let out = tool
            .execute(
                json!({"url": url, "sql": "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)", "write": true}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.contains("OK"), "{}", out);

        let out = tool
            .execute(
                json!({"url": url, "sql": "INSERT INTO t (name) VALUES ('alpha'), ('beta')", "write": true}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.contains("2 row(s) affected"), "{}", out);

        // Read back without the write flag.
        let out = tool
            .execute(
                json!({"url": url, "sql": "SELECT id, name FROM t ORDER BY id"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.contains("2 row(s)"), "{}", out);
        assert!(out.contains("alpha") && out.contains("beta"), "{}", out);
    }

    #[tokio::test]
    async fn db_query_rejects_multiple_statements() {
        let db = temp_db("multi");
        let tool = DbQueryTool;
        let err = tool
            .execute(
                json!({"url": format!("sqlite://{}", db.display()), "sql": "SELECT 1; DROP TABLE t"}),
                &ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("one statement"), "{}", err);
    }

    #[tokio::test]
    async fn db_query_write_sneaking_past_keyword_check_is_still_blocked() {
        // A CTE that ends in an UPDATE: starts with "WITH" but is not readonly.
        let db = temp_db("cte");
        let tool = DbQueryTool;
        let url = format!("sqlite://{}", db.display());
        let _ = tool
            .execute(
                json!({"url": url, "sql": "CREATE TABLE t (id INTEGER)", "write": true}),
                &ToolContext::default(),
            )
            .await;
        let err = tool
            .execute(
                json!({"url": url, "sql": "WITH x AS (SELECT 1) UPDATE t SET id = 1"}),
                &ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("write: true"), "{}", err);
    }

    #[tokio::test]
    async fn db_query_readonly_cannot_open_missing_db() {
        let tool = DbQueryTool;
        let err = tool
            .execute(
                json!({"url": "sqlite:///definitely/not/here.db", "sql": "SELECT 1"}),
                &ToolContext::default(),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("failed to open"), "{}", err);
    }
}
