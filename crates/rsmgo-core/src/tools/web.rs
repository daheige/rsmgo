use crate::error::{Result, RsmgoError};
use crate::tools::Tool;
use serde_json::json;
use std::process::Command;

/// A single search result extracted from the DuckDuckGo HTML endpoint.
struct SearchResult {
    title: String,
    url: String,
    snippet: String,
}

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

/// Fetch a URL via `curl` and return the raw body. `curl` is used because the
/// `Tool::execute` trait is synchronous and the blocking reqwest feature is not
/// enabled in this crate.
fn fetch(url: &str) -> Result<String> {
    let output = Command::new("curl")
        .args(["-sL", "--max-time", "20", "-A", USER_AGENT, url])
        .output()
        .map_err(|e| RsmgoError::Tool(format!("curl failed: {}", e)))?;
    if !output.status.success() {
        return Err(RsmgoError::Tool(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub struct WebSearchTool;

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web using DuckDuckGo and return the top results (title, URL and snippet)."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "The search query" }
            },
            "required": ["query"]
        })
    }

    fn execute(&self, args: serde_json::Value) -> Result<String> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'query' argument".to_string()))?;
        let html = fetch(&format!(
            "https://html.duckduckgo.com/html/?q={}",
            urlencode(query)
        ))?;
        let results = parse_duckduckgo(&html);
        if results.is_empty() {
            return Ok("No results found.".to_string());
        }

        let mut out = String::new();
        for (i, r) in results.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, r.title));
            if !r.url.is_empty() {
                out.push_str(&format!("   URL: {}\n", r.url));
            }
            if !r.snippet.is_empty() {
                out.push_str(&format!("   {}\n", r.snippet));
            }
            out.push('\n');
        }
        Ok(out.trim_end().to_string())
    }
}

pub struct FetchUrlTool;

impl Tool for FetchUrlTool {
    fn name(&self) -> &str {
        "fetch_url"
    }

    fn description(&self) -> &str {
        "Fetch the contents of a web page and return its text, stripped of HTML tags."
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The URL to fetch" }
            },
            "required": ["url"]
        })
    }

    fn execute(&self, args: serde_json::Value) -> Result<String> {
        let url = args["url"]
            .as_str()
            .ok_or_else(|| RsmgoError::Tool("missing 'url' argument".to_string()))?;
        let html = fetch(url)?;
        let text = strip_tags(&html);
        let text = text.trim();
        const MAX_LEN: usize = 6000;
        let text = if text.chars().count() > MAX_LEN {
            let mut t: String = text.chars().take(MAX_LEN).collect();
            t.push_str("\n...(truncated)");
            t
        } else {
            text.to_string()
        };
        Ok(text)
    }
}

/// Parse the DuckDuckGo HTML results page into a list of `SearchResult`s.
/// DuckDuckGo renders each result as a `result__a` anchor (title + href)
/// followed by a `result__snippet` anchor (description).
fn parse_duckduckgo(html: &str) -> Vec<SearchResult> {
    let mut titles: Vec<String> = Vec::new();
    let mut urls: Vec<String> = Vec::new();
    let mut search_from = 0usize;

    while let Some(rel) = html[search_from..].find("result__a") {
        let pos = search_from + rel;
        let seg = &html[pos..];

        let url = find_between(seg, "href=\"", "\"")
            .map(clean_ddg_url)
            .unwrap_or_default();
        let title = seg
            .find('>')
            .and_then(|i| {
                let after = &seg[i + 1..];
                let end = after.find("</a>")?;
                Some(decode_entities(&strip_tags(&after[..end])).trim().to_string())
            })
            .unwrap_or_default();

        titles.push(title);
        urls.push(url);
        search_from = pos + "result__a".len();
    }

    let mut snippets: Vec<String> = Vec::new();
    search_from = 0usize;
    while let Some(rel) = html[search_from..].find("result__snippet") {
        let pos = search_from + rel;
        let seg = &html[pos..];
        let snippet = seg
            .find('>')
            .and_then(|i| {
                let after = &seg[i + 1..];
                let end = after.find("</a>")?;
                Some(decode_entities(&strip_tags(&after[..end])).trim().to_string())
            })
            .unwrap_or_default();
        snippets.push(snippet);
        search_from = pos + "result__snippet".len();
    }

    titles
        .into_iter()
        .enumerate()
        .take(8)
        .map(|(i, title)| SearchResult {
            title,
            url: urls.get(i).cloned().unwrap_or_default(),
            snippet: snippets.get(i).cloned().unwrap_or_default(),
        })
        .collect()
}

/// Extract the real target URL from a DuckDuckGo redirect `uddg=` parameter,
/// falling back to the raw href otherwise.
fn clean_ddg_url(href: &str) -> String {
    if let Some(idx) = href.find("uddg=") {
        let encoded = &href[idx + 5..];
        let encoded = encoded.split('&').next().unwrap_or("");
        return percent_decode(encoded);
    }
    href.strip_prefix("//").unwrap_or(href).to_string()
}

fn find_between<'a>(haystack: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let s = haystack.find(start)? + start.len();
    let e = haystack[s..].find(end)? + s;
    Some(&haystack[s..e])
}

fn strip_tags(input: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in input.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

fn decode_entities(input: &str) -> String {
    input
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn urlencode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}
