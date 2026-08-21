"use client";

import { useEffect, useRef, useState } from "react";
import * as api from "@/lib/api";

const WEB_SEARCH_TOOL = "web_search";

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
  const bottomRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const toolsInitialized = useRef(false);

  useEffect(() => {
    if (!sessionId) return;
    api.getSession(sessionId).then((s) => setMessages(s.messages ?? [])).catch(() => setMessages([]));
    setAttachments([]);
  }, [sessionId]);

  useEffect(() => {
    if (toolsInitialized.current || tools.length === 0) return;
    setEnabledTools(tools.filter((t) => t.name !== WEB_SEARCH_TOOL).map((t) => t.name));
    toolsInitialized.current = true;
  }, [tools]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, loading]);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    const nextHeight = Math.min(el.scrollHeight, 120);
    el.style.height = `${Math.max(nextHeight, 44)}px`;
  }, [input]);

  const canSend = !loading && (input.trim().length > 0 || attachments.length > 0);

  const send = async () => {
    if (!sessionId || !canSend) return;
    const text = input.trim();
    const display = text || attachments.map((a) => `[${a.name}]`).join(" ");
    const userMsg: api.Message = { role: "user", content: display };
    setMessages((prev) => [...prev, userMsg]);
    setInput("");
    setLoading(true);
    setError(null);
    const attachmentIds = attachments.map((a) => a.id);
    setAttachments([]);
    try {
      const resp = await api.chat(sessionId, text, {
        toolNames: enabledTools,
        webSearch,
        attachmentIds,
      });
      if (resp.message) {
        setMessages((prev) => [...prev, resp.message!]);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
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
        {messages.map((m, i) => (
          <div
            key={i}
            style={{
              ...styles.message,
              alignSelf: m.role === "user" ? "flex-end" : "flex-start",
              background: m.role === "user" ? "#2563eb" : "#1e293b",
            }}
          >
            {m.content}
          </div>
        ))}
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
                  title="移除"
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
          style={styles.input}
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
            title="上传文件"
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
            title="联网搜索"
            disabled={!sessionId}
            onClick={() => setWebSearch((v) => !v)}
          >
            🌐
          </button>
          <div style={styles.toolsWrap}>
            <button
              style={styles.iconButton}
              title="更多工具"
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
          <span style={styles.spacer} />
          <button style={styles.button} onClick={send} disabled={!sessionId || !canSend}>
            Send
          </button>
        </div>
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
  },
  message: {
    maxWidth: "80%",
    padding: "0.75rem 1rem",
    borderRadius: "0.5rem",
    whiteSpace: "pre-wrap",
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
    minHeight: "44px",
    maxHeight: "120px",
    padding: "0.75rem 1rem",
    borderRadius: "0.5rem",
    border: "1px solid #334155",
    background: "#0f172a",
    color: "#e2e8f0",
    resize: "none",
    outline: "none",
    lineHeight: "1.25rem",
    boxSizing: "border-box",
  },
  toolbar: {
    display: "flex",
    alignItems: "center",
    gap: "0.4rem",
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
};
