"use client";

import { useEffect, useState } from "react";
import Chat from "@/components/Chat";
import * as api from "@/lib/api";

export default function Home() {
  const [sessions, setSessions] = useState<api.Session[]>([]);
  const [currentId, setCurrentId] = useState<string | undefined>();
  const [providers, setProviders] = useState<string[]>([]);
  const [tools, setTools] = useState<api.ToolInfo[]>([]);
  const [title, setTitle] = useState("");
  const [provider, setProvider] = useState("openai");
  const [model, setModel] = useState("");
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editTitle, setEditTitle] = useState("");

  useEffect(() => {
    loadSessions();
    api.listProviders().then((ps) => {
      setProviders(ps);
      if (ps.length) setProvider(ps[0]);
    }).catch(console.error);
    api.listTools().then(setTools).catch(console.error);
  }, []);

  const loadSessions = () => {
    api.listSessions().then(setSessions).catch(console.error);
  };

  const createSession = async () => {
    const t = title.trim() || "New chat";
    const s = await api.createSession({ title: t, provider, model });
    setTitle("");
    loadSessions();
    setCurrentId(s.id);
  };

  const togglePin = async (s: api.Session) => {
    try {
      await api.updateSession(s.id, { pinned: !s.pinned });
      loadSessions();
    } catch (e) {
      console.error(e);
    }
  };

  const startRename = (s: api.Session) => {
    setEditingId(s.id);
    setEditTitle(s.title);
  };

  const saveRename = async () => {
    if (!editingId) return;
    const id = editingId;
    setEditingId(null);
    try {
      await api.updateSession(id, { title: editTitle.trim() || "New chat" });
      loadSessions();
    } catch (e) {
      console.error(e);
    }
  };

  const removeSession = async (s: api.Session) => {
    if (!window.confirm(`Delete session "${s.title}"?`)) return;
    try {
      await api.deleteSession(s.id);
      if (currentId === s.id) setCurrentId(undefined);
      loadSessions();
    } catch (e) {
      console.error(e);
    }
  };

  return (
    <div style={styles.layout}>
      <aside style={styles.sidebar}>
        <div style={styles.header}>
          <h1 style={styles.logo}>rsmgo</h1>
          <button style={styles.newChat} onClick={createSession}>＋ New chat</button>
        </div>

        <div style={styles.newSession}>
          <input
            style={styles.input}
            placeholder="Session title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
          />
          <div style={styles.row}>
            <select style={styles.select} value={provider} onChange={(e) => setProvider(e.target.value)}>
              {providers.map((p) => (
                <option key={p} value={p}>{p}</option>
              ))}
            </select>
            <input
              style={{ ...styles.input, flex: 1 }}
              placeholder="Model (optional)"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            />
          </div>
        </div>

        <ul style={styles.sessionList}>
          {sessions.map((s) => (
            <li
              key={s.id}
              style={{
                ...styles.sessionItem,
                background: s.id === currentId ? "#334155" : "transparent",
              }}
              onClick={() => setCurrentId(s.id)}
            >
              <div style={styles.sessionRow}>
                {s.pinned && <span style={styles.pinBadge}>📌</span>}
                {editingId === s.id ? (
                  <input
                    autoFocus
                    style={styles.renameInput}
                    value={editTitle}
                    onChange={(e) => setEditTitle(e.target.value)}
                    onBlur={saveRename}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") saveRename();
                      if (e.key === "Escape") setEditingId(null);
                    }}
                    onClick={(e) => e.stopPropagation()}
                  />
                ) : (
                  <span style={styles.sessionTitle}>{s.title}</span>
                )}
                <div style={styles.actions} onClick={(e) => e.stopPropagation()}>
                  <button
                    style={{ ...styles.action, opacity: s.pinned ? 1 : 0.55 }}
                    title={s.pinned ? "取消置顶" : "置顶"}
                    onClick={() => togglePin(s)}
                  >
                    📌
                  </button>
                  <button style={styles.action} title="编辑标题" onClick={() => startRename(s)}>
                    ✏️
                  </button>
                  <button style={styles.action} title="删除会话" onClick={() => removeSession(s)}>
                    🗑
                  </button>
                </div>
              </div>
              <div style={styles.meta}>{s.provider} / {s.model || "default"}</div>
            </li>
          ))}
        </ul>

        <div style={styles.tools}>
          <strong>Tools ({tools.length})</strong>
        </div>
      </aside>
      <main style={styles.main}>
        {currentId ? (
          <Chat sessionId={currentId} tools={tools} />
        ) : (
          <div style={styles.empty}>Select or create a session to start chatting.</div>
        )}
      </main>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  layout: {
    display: "flex",
    height: "100vh",
  },
  sidebar: {
    width: "280px",
    background: "#1e293b",
    padding: "1rem",
    display: "flex",
    flexDirection: "column",
    gap: "1rem",
    borderRight: "1px solid #334155",
  },
  header: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
  },
  logo: {
    fontSize: "1.5rem",
    fontWeight: 700,
    color: "#60a5fa",
  },
  newChat: {
    padding: "0.5rem 0.75rem",
    borderRadius: "0.375rem",
    border: "none",
    background: "#2563eb",
    color: "#fff",
    cursor: "pointer",
    whiteSpace: "nowrap",
  },
  newSession: {
    display: "flex",
    flexDirection: "column",
    gap: "0.5rem",
  },
  row: {
    display: "flex",
    gap: "0.5rem",
  },
  input: {
    padding: "0.5rem",
    borderRadius: "0.375rem",
    border: "1px solid #334155",
    background: "#0f172a",
    color: "#e2e8f0",
    minWidth: 0,
  },
  select: {
    padding: "0.5rem",
    borderRadius: "0.375rem",
    border: "1px solid #334155",
    background: "#0f172a",
    color: "#e2e8f0",
  },
  sessionList: {
    listStyle: "none",
    display: "flex",
    flexDirection: "column",
    gap: "0.25rem",
    overflowY: "auto",
    flex: 1,
    margin: 0,
    padding: 0,
  },
  sessionItem: {
    padding: "0.5rem",
    borderRadius: "0.375rem",
    cursor: "pointer",
  },
  sessionRow: {
    display: "flex",
    alignItems: "center",
    gap: "0.25rem",
  },
  pinBadge: {
    fontSize: "0.75rem",
  },
  sessionTitle: {
    flex: 1,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  renameInput: {
    flex: 1,
    padding: "0.25rem",
    borderRadius: "0.25rem",
    border: "1px solid #60a5fa",
    background: "#0f172a",
    color: "#e2e8f0",
    minWidth: 0,
  },
  actions: {
    display: "flex",
    gap: "0.15rem",
  },
  action: {
    border: "none",
    background: "transparent",
    color: "#e2e8f0",
    cursor: "pointer",
    padding: "0.1rem",
    fontSize: "0.85rem",
    lineHeight: 1,
  },
  meta: {
    fontSize: "0.75rem",
    color: "#94a3b8",
    paddingLeft: "1.15rem",
    marginTop: "0.15rem",
  },
  tools: {
    fontSize: "0.875rem",
    color: "#94a3b8",
  },
  main: {
    flex: 1,
    padding: "1rem",
    display: "flex",
    flexDirection: "column",
  },
  empty: {
    flex: 1,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    color: "#94a3b8",
  },
};
