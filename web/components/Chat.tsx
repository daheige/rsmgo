"use client";

import { Children, useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import hljs from "highlight.js";
import "highlight.js/styles/github-dark.css";
import * as api from "@/lib/api";

const WEB_SEARCH_TOOL = "web_search";

interface DownloadLink {
  name: string;
  url: string;
}

// Some providers return escape sequences (\n, \t, \") literally inside message
// content. Convert them back to real characters so Markdown renders correctly.
const unescapeContent = (content: string): string => {
  return content
    .replace(/\\n/g, "\n")
    .replace(/\\t/g, "\t")
    .replace(/\\"/g, '"')
    .replace(/\\\\/g, "\\");
};

// Rewrite model-generated file links to the canonical download endpoint.
// Models often paraphrase the tool result and emit `[text](my.md)`,
// `[text](/api/files/my.md)`, or `[text](/api/v1/files/my.md)`; this normalizes
// them to `/api/v1/files/<basename>`.
const normalizeFileLinks = (content: string): string => {
  return content.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (match, text, href) => {
    const trimmed = href.trim();
    // Leave external URLs untouched.
    if (/^https?:\/\//i.test(trimmed)) {
      return match;
    }
    // Canonicalize file download links: strip query/fragment and keep only the base name.
    const filePrefix = trimmed.startsWith("/api/v1/files/")
      ? "/api/v1/files/"
      : trimmed.startsWith("/api/files/")
        ? "/api/files/"
        : "";
    if (filePrefix) {
      const rawName = trimmed.slice(filePrefix.length);
      const baseName = rawName.split(/[?#]/)[0];
      if (baseName) {
        return `[${text}](/api/v1/files/${encodeURIComponent(baseName)})`;
      }
      return match;
    }
    // Leave other internal API routes untouched.
    if (trimmed.startsWith("/api/") || trimmed.startsWith("/health")) {
      return match;
    }
    const base = trimmed.replace(/^\/+/, "").split("/").pop();
    if (base && base.includes(".")) {
      return `[${text}](/api/v1/files/${encodeURIComponent(base)})`;
    }
    return match;
  });
};

const extractDownloads = (content: string): DownloadLink[] => {
  const links: DownloadLink[] = [];
  const linkRegex = /\[([^\]]+)\]\(([^)]+)\)/g;
  let match: RegExpExecArray | null;
  while ((match = linkRegex.exec(content)) !== null) {
    const href = match[2].trim();
    if (/^https?:\/\//i.test(href)) continue;
    const isFile = /^\/api(?:\/v1)?\/files\//.test(href);
    const isWorkspaceFile = /^\/api\/v1\/workspaces\/[^/]+\/files\//.test(href);
    if (!isFile && !isWorkspaceFile) continue;
    const cleanPath = href.split(/[?#]/)[0];
    const fileName = decodeURIComponent(cleanPath.split("/").pop() || "");
    links.push({ name: fileName, url: cleanPath });
  }
  return links;
};

const CopyIcon = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
  </svg>
);

const CheckIcon = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <polyline points="20 6 9 17 4 12" />
  </svg>
);

const RetryIcon = () => (
  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
    <polyline points="23 4 23 10 17 10" />
    <polyline points="1 20 1 14 7 14" />
    <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
  </svg>
);

// Copy arbitrary text to the clipboard, falling back to a hidden textarea for
// non-secure contexts (e.g. the Tauri file:// origin).
async function writeClipboard(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.opacity = "0";
  document.body.appendChild(ta);
  ta.select();
  document.execCommand("copy");
  document.body.removeChild(ta);
}

interface CodeBlockProps extends React.HTMLAttributes<HTMLPreElement> {
  node?: unknown;
}

// Fenced code block wrapper: renders a language label + copy button header
// above the highlighted <pre>. The raw text is read from the DOM at copy time
// so highlighted (span-wrapped) code still copies as plain source.
function CodeBlock({ children, node: _node, ...rest }: CodeBlockProps) {
  const preRef = useRef<HTMLPreElement>(null);
  const [copied, setCopied] = useState(false);

  const child = Children.toArray(children)[0] as
    | React.ReactElement<{ className?: string }>
    | undefined;
  const lang = /language-([\w+-]+)/.exec(child?.props?.className ?? "")?.[1] ?? "code";

  const copyCode = async () => {
    try {
      await writeClipboard(preRef.current?.textContent ?? "");
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      // Ignore copy failures; nothing actionable to surface in this context.
    }
  };

  return (
    <div className="code-block">
      <div className="code-block-header">
        <span className="code-lang">{lang}</span>
        <button
          className="msg-action"
          onClick={copyCode}
          title={copied ? "Copied" : "Copy code"}
          aria-label="Copy code"
        >
          {copied ? <CheckIcon /> : <CopyIcon />}
        </button>
      </div>
      <pre ref={preRef} {...rest}>
        {children}
      </pre>
    </div>
  );
}

interface ChatProps {
  sessionId?: string;
  tools?: api.ToolInfo[];
}

export default function Chat({ sessionId, tools = [] }: ChatProps) {
  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<api.Message[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [attachments, setAttachments] = useState<api.Attachment[]>([]);
  const [enabledTools, setEnabledTools] = useState<string[]>([]);
  const [webSearch, setWebSearch] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(false);
  const [copiedIndex, setCopiedIndex] = useState<number | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const toolsInitialized = useRef(false);
  const abortRef = useRef<AbortController | null>(null);

  useEffect(() => {
    if (!sessionId) return;
    api.getSession(sessionId).then((s) => setMessages(s.messages ?? [])).catch(() => setMessages([]));
    setAttachments([]);
  }, [sessionId]);

  useEffect(() => {
    if (toolsInitialized.current) return;
    // Enable all selectable tools by default so file/command operations work
    // out of the box. Users can still uncheck tools in the menu if needed.
    setEnabledTools(selectableTools.map((t) => t.name));
    toolsInitialized.current = true;
  }, [tools]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, loading]);

  // Reset the textarea to the default height on mount so the browser does not
  // restore a previously resized height.
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "200px";
  }, []);

  const canSend = !loading && (input.trim().length > 0 || attachments.length > 0);

  const submitChat = async (content: string, opts: api.ChatOptions) => {
    if (!sessionId) return;
    setLoading(true);
    setError(null);
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      const resp = await api.chat(sessionId, content, opts, controller.signal);
      if (resp.message) {
        setMessages((prev) => [...prev, resp.message!]);
      }
    } catch (e) {
      if (e instanceof DOMException && e.name === "AbortError") {
        // User cancelled generation; do not surface as an error.
      } else {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (abortRef.current === controller) abortRef.current = null;
      setLoading(false);
    }
  };

  const send = async () => {
    if (!sessionId || !canSend) return;
    const text = input.trim();
    const display = text || attachments.map((a) => `[${a.name}]`).join(" ");
    const userMsg: api.Message = { role: "user", content: display };
    setMessages((prev) => [...prev, userMsg]);
    setInput("");
    const attachmentIds = attachments.map((a) => a.id);
    setAttachments([]);
    await submitChat(text, { toolNames: enabledTools, webSearch, attachmentIds });
  };

  const retry = async (assistantIndex: number) => {
    if (!sessionId || loading) return;
    const userContent = messages
      .slice(0, assistantIndex)
      .filter((m) => m.role === "user")
      .pop()?.content;
    if (!userContent) return;
    // Drop the assistant message being retried (and anything after it) so the
    // new response replaces it in place.
    setMessages((prev) => prev.slice(0, assistantIndex));
    await submitChat(userContent, { toolNames: enabledTools, webSearch, regenerate: true });
  };

  const copyMessage = async (m: api.Message, index: number) => {
    try {
      await writeClipboard(m.content);
      setCopiedIndex(index);
      window.setTimeout(() => {
        setCopiedIndex((cur) => (cur === index ? null : cur));
      }, 1500);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  // Abort the in-flight request and ask the control plane to cancel the
  // engine-side chat so generation stops promptly.
  const stop = () => {
    if (!sessionId) return;
    abortRef.current?.abort();
    api.cancelChat(sessionId).catch(() => {});
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  const handleFiles = async (files: FileList | null) => {
    if (!files || files.length === 0) return;
    for (const file of Array.from(files)) {
      try {
        const att = await api.uploadFile(file);
        setAttachments((prev) => [...prev, att]);
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    }
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  const toggleTool = (name: string) => {
    setEnabledTools((prev) =>
      prev.includes(name) ? prev.filter((n) => n !== name) : [...prev, name],
    );
  };

  const selectableTools = tools.filter((t) => t.name !== WEB_SEARCH_TOOL);

  return (
    <div style={styles.container}>
      <div style={styles.messages}>
        {messages.map((m, i) => {
          const displayContent =
            m.role === "assistant"
              ? normalizeFileLinks(unescapeContent(m.content))
              : m.content;
          const downloads =
            m.role === "assistant" ? extractDownloads(displayContent) : [];
          return (
            <div
              key={i}
              style={{
                ...styles.message,
                alignSelf: m.role === "user" ? "flex-end" : "flex-start",
                background: m.role === "user" ? "#2563eb" : "#1e293b",
              }}
            >
              <div className="markdown-content">
                <ReactMarkdown
                  remarkPlugins={[remarkGfm]}
                  components={{
                    pre: ({ children, node, ...rest }) => (
                      <CodeBlock node={node} {...rest}>
                        {children}
                      </CodeBlock>
                    ),
                    code: ({ className, children, node: _node, ...rest }) => {
                      const match = /language-([\w+-]+)/.exec(className || "");
                      if (match) {
                        const lang = match[1];
                        const code = String(children).replace(/\n$/, "");
                        if (hljs.getLanguage(lang)) {
                          try {
                            const html = hljs.highlight(code, {
                              language: lang,
                              ignoreIllegals: true,
                            }).value;
                            return (
                              <code
                                className={`hljs ${className ?? ""}`}
                                dangerouslySetInnerHTML={{ __html: html }}
                                {...rest}
                              />
                            );
                          } catch {
                            // Fall through to plain rendering on unexpected errors.
                          }
                        }
                      }
                      return (
                        <code className={className} {...rest}>
                          {children}
                        </code>
                      );
                    },
                    a: ({ href, children }) => {
                      const isDownload =
                        typeof href === "string" &&
                        (href.startsWith("/api/v1/files/") ||
                          href.startsWith("/api/files/") ||
                          /^\/api\/v1\/workspaces\/[^/]+\/files\//.test(href));
                      let downloadName: string | undefined;
                      if (isDownload && typeof href === "string") {
                        const base = href
                          .replace(/^\/api(?:\/v1)?\/files\//, "")
                          .replace(/^\/api\/v1\/workspaces\/[^/]+\/files\//, "")
                          .split(/[?#]/)[0];
                        downloadName = decodeURIComponent(base);
                      }
                      return (
                        <a
                          href={href}
                          download={downloadName}
                          style={{ color: "#60a5fa" }}
                        >
                          {children}
                        </a>
                      );
                    },
                  }}
                >
                  {displayContent}
                </ReactMarkdown>
              </div>
              {downloads.length > 0 && (
                <div style={styles.downloads}>
                  {downloads.map((d) => (
                    <a key={d.url} href={d.url} download={d.name} style={styles.downloadBtn}>
                      ⬇ Download {d.name}
                    </a>
                  ))}
                </div>
              )}
              {m.role === "assistant" && (
                <div className="msg-actions">
                  <button
                    className="msg-action"
                    onClick={() => copyMessage(m, i)}
                    title={copiedIndex === i ? "Copied" : "Copy"}
                    aria-label="Copy"
                  >
                    {copiedIndex === i ? <CheckIcon /> : <CopyIcon />}
                  </button>
                  {i === messages.length - 1 && (
                    <button
                      className="msg-action"
                      onClick={() => retry(i)}
                      title="Retry"
                      aria-label="Retry"
                      disabled={loading}
                    >
                      <RetryIcon />
                    </button>
                  )}
                </div>
              )}
            </div>
          );
        })}
        {loading && <div style={styles.typing}>Thinking...</div>}
        {error && <div style={styles.error}>{error}</div>}
        <div ref={bottomRef} />
      </div>

      <div style={styles.composer}>
        {attachments.length > 0 && (
          <div style={styles.chips}>
            {attachments.map((a) => (
              <span key={a.id} style={styles.chip}>
                {a.content_type.startsWith("image/") ? (
                  <img src={api.attachmentUrl(a.id)} alt={a.name} style={styles.thumb} />
                ) : (
                  <span style={styles.chipIcon}>📄</span>
                )}
                <span style={styles.chipName}>{a.name}</span>
                <button
                  style={styles.chipRemove}
                  title="Remove"
                  onClick={() => setAttachments((prev) => prev.filter((x) => x.id !== a.id))}
                >
                  ✕
                </button>
              </span>
            ))}
          </div>
        )}

        <textarea
          ref={textareaRef}
          style={{ ...styles.input, height: "200px" }}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Type a message..."
          rows={1}
          disabled={!sessionId}
        />

        <div style={styles.toolbar}>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            style={{ display: "none" }}
            onChange={(e) => handleFiles(e.target.files)}
          />
          <button
            style={styles.iconButton}
            title="Upload file"
            disabled={!sessionId}
            onClick={() => fileInputRef.current?.click()}
          >
            📎
          </button>
          <button
            style={{
              ...styles.iconButton,
              ...(webSearch ? styles.iconButtonActive : {}),
            }}
            title="Web search"
            disabled={!sessionId}
            onClick={() => setWebSearch((v) => !v)}
          >
            🌐
          </button>
          <div style={styles.toolsWrap}>
            <button
              style={styles.iconButton}
              title="More tools"
              disabled={!sessionId}
              onClick={() => setToolsOpen((v) => !v)}
            >
              🛠
            </button>
            {toolsOpen && (
              <div style={styles.toolsMenu}>
                <div style={styles.toolsMenuHeader}>Tools</div>
                {selectableTools.map((t) => (
                  <label key={t.name} style={styles.toolOption} title={t.description}>
                    <input
                      type="checkbox"
                      checked={enabledTools.includes(t.name)}
                      onChange={() => toggleTool(t.name)}
                    />
                    <span style={styles.toolName}>{t.name}</span>
                  </label>
                ))}
              </div>
            )}
          </div>
        </div>
        {loading ? (
          <button
            style={{
              ...styles.button,
              ...styles.stopButton,
              position: "absolute",
              right: "0.75rem",
              bottom: "0.75rem",
            }}
            onClick={stop}
            title="Stop generation"
          >
            Stop
          </button>
        ) : (
          <button
            style={{
              ...styles.button,
              position: "absolute",
              right: "0.75rem",
              bottom: "0.75rem",
            }}
            onClick={send}
            disabled={!sessionId || !canSend}
          >
            Send
          </button>
        )}
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    gap: "1rem",
    minWidth: 0,
  },
  messages: {
    flex: 1,
    overflowY: "auto",
    display: "flex",
    flexDirection: "column",
    gap: "0.75rem",
    padding: "1rem",
    background: "#020617",
    borderRadius: "0.5rem",
    minWidth: 0,
  },
  message: {
    maxWidth: "100%",
    padding: "0.75rem 1rem",
    borderRadius: "0.5rem",
    wordBreak: "break-word",
  },
  typing: {
    color: "#94a3b8",
    fontStyle: "italic",
  },
  error: {
    color: "#f87171",
  },
  composer: {
    display: "flex",
    flexDirection: "column",
    gap: "0.5rem",
    padding: "0.75rem",
    background: "#1e293b",
    borderRadius: "0.75rem",
    position: "relative",
    minWidth: 0,
  },
  chips: {
    display: "flex",
    flexWrap: "wrap",
    gap: "0.5rem",
  },
  chip: {
    display: "inline-flex",
    alignItems: "center",
    gap: "0.4rem",
    padding: "0.25rem 0.5rem",
    background: "#0f172a",
    border: "1px solid #334155",
    borderRadius: "0.5rem",
    fontSize: "0.8rem",
    color: "#e2e8f0",
    maxWidth: "220px",
  },
  chipIcon: {
    fontSize: "0.9rem",
  },
  downloadBtn: {
    display: "inline-flex",
    alignItems: "center",
    gap: "0.35rem",
    padding: "0.35rem 0.75rem",
    borderRadius: "0.375rem",
    background: "#059669",
    color: "#fff",
    textDecoration: "none",
    fontSize: "0.85rem",
    fontWeight: 500,
  },
  downloads: {
    display: "flex",
    flexWrap: "wrap",
    gap: "0.5rem",
    marginTop: "0.75rem",
    paddingTop: "0.75rem",
    borderTop: "1px solid rgba(255,255,255,0.1)",
  },
  thumb: {
    width: "24px",
    height: "24px",
    objectFit: "cover",
    borderRadius: "0.25rem",
  },
  chipName: {
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  chipRemove: {
    border: "none",
    background: "transparent",
    color: "#94a3b8",
    cursor: "pointer",
    padding: 0,
    fontSize: "0.8rem",
  },
  input: {
    width: "100%",
    minWidth: "0",
    maxWidth: "100%",
    minHeight: "200px",
    padding: "0.75rem 1rem 3.75rem 1rem",
    borderRadius: "0.5rem",
    border: "1px solid #334155",
    background: "#0f172a",
    color: "#e2e8f0",
    resize: "both",
    overflow: "auto",
    outline: "none",
    lineHeight: "1.25rem",
    boxSizing: "border-box",
  },
  toolbar: {
    display: "flex",
    alignItems: "center",
    gap: "0.4rem",
    flexWrap: "nowrap",
    flexShrink: 0,
    minWidth: 0,
  },
  iconButton: {
    border: "1px solid #334155",
    background: "#0f172a",
    color: "#cbd5e1",
    cursor: "pointer",
    width: "36px",
    height: "36px",
    borderRadius: "0.5rem",
    fontSize: "1rem",
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
  },
  iconButtonActive: {
    borderColor: "#60a5fa",
    color: "#60a5fa",
    background: "#1e3a5f",
  },
  toolsWrap: {
    position: "relative",
  },
  toolsMenu: {
    position: "absolute",
    bottom: "44px",
    left: 0,
    width: "220px",
    background: "#0f172a",
    border: "1px solid #334155",
    borderRadius: "0.5rem",
    padding: "0.5rem",
    display: "flex",
    flexDirection: "column",
    gap: "0.25rem",
    zIndex: 10,
    boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
  },
  toolsMenuHeader: {
    fontSize: "0.75rem",
    color: "#94a3b8",
    textTransform: "uppercase",
    paddingBottom: "0.25rem",
  },
  toolOption: {
    display: "flex",
    alignItems: "center",
    gap: "0.5rem",
    padding: "0.25rem",
    cursor: "pointer",
    borderRadius: "0.25rem",
  },
  toolName: {
    fontSize: "0.85rem",
    color: "#e2e8f0",
  },
  spacer: {
    flex: 1,
  },
  button: {
    padding: "0.5rem 1.25rem",
    borderRadius: "0.5rem",
    border: "none",
    background: "#2563eb",
    color: "#fff",
    cursor: "pointer",
    height: "36px",
  },
  stopButton: {
    background: "#dc2626",
  },
};
