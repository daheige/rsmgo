"use client";

import { useEffect, useRef, useState } from "react";
import * as api from "@/lib/api";

interface ChatProps {
  sessionId?: string;
}

export default function Chat({ sessionId }: ChatProps) {
  const [input, setInput] = useState("");
  const [messages, setMessages] = useState<api.Message[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    if (!sessionId) return;
    api.getSession(sessionId).then((s) => setMessages(s.messages ?? [])).catch(() => setMessages([]));
  }, [sessionId]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    const nextHeight = Math.min(el.scrollHeight, 120);
    el.style.height = `${Math.max(nextHeight, 44)}px`;
  }, [input]);

  const send = async () => {
    if (!sessionId || !input.trim()) return;
    const userMsg: api.Message = { role: "user", content: input };
    setMessages((prev) => [...prev, userMsg]);
    setInput("");
    setLoading(true);
    setError(null);
    try {
      const resp = await api.chat(sessionId, userMsg.content);
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
      <div style={styles.inputRow}>
        <textarea
          ref={textareaRef}
          style={styles.input}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="Type a message..."
          rows={1}
          disabled={!sessionId || loading}
        />
        <button style={styles.button} onClick={send} disabled={!sessionId || loading}>
          Send
        </button>
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
  inputRow: {
    display: "flex",
    gap: "0.5rem",
    alignItems: "flex-end",
  },
  input: {
    flex: 1,
    minHeight: "44px",
    maxHeight: "120px",
    padding: "0.75rem 1rem",
    borderRadius: "0.5rem",
    border: "1px solid #334155",
    background: "#1e293b",
    color: "#e2e8f0",
    resize: "none",
    outline: "none",
    lineHeight: "1.25rem",
  },
  button: {
    padding: "0.75rem 1.5rem",
    borderRadius: "0.5rem",
    border: "none",
    background: "#2563eb",
    color: "#fff",
    cursor: "pointer",
    height: "44px",
  },
};
