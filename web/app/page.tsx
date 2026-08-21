"use client";

import { useEffect, useState } from "react";
import Chat from "@/components/Chat";
import * as api from "@/lib/api";

export default function Home() {
  const [sessions, setSessions] = useState<api.Session[]>([]);
  const [currentId, setCurrentId] = useState<string | undefined>();
  const [providers, setProviders] = useState<string[]>([]);
  const [tools, setTools] = useState<unknown[]>([]);
  const [title, setTitle] = useState("");
  const [provider, setProvider] = useState("openai");
  const [model, setModel] = useState("");

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
    setSessions((prev) => [s, ...prev]);
    setCurrentId(s.id);
  };

  return (
    <div style={styles.layout}>
      <aside style={styles.sidebar}>
        <h1 style={styles.logo}>rsmgo</h1>
        <div style={styles.newSession}>
          <input
            style={styles.input}
            placeholder="Session title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
          />
          <select style={styles.select} value={provider} onChange={(e) => setProvider(e.target.value)}>
            {providers.map((p) => (
              <option key={p} value={p}>{p}</option>
            ))}
          </select>
          <input
            style={styles.input}
            placeholder="Model (optional)"
            value={model}
            onChange={(e) => setModel(e.target.value)}
          />
          <button style={styles.button} onClick={createSession}>New Session</button>
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
              <div>{s.title}</div>
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
          <Chat sessionId={currentId} />
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
  logo: {
    fontSize: "1.5rem",
    fontWeight: 700,
    color: "#60a5fa",
  },
  newSession: {
    display: "flex",
    flexDirection: "column",
    gap: "0.5rem",
  },
  input: {
    padding: "0.5rem",
    borderRadius: "0.375rem",
    border: "1px solid #334155",
    background: "#0f172a",
    color: "#e2e8f0",
  },
  select: {
    padding: "0.5rem",
    borderRadius: "0.375rem",
    border: "1px solid #334155",
    background: "#0f172a",
    color: "#e2e8f0",
  },
  button: {
    padding: "0.5rem",
    borderRadius: "0.375rem",
    border: "none",
    background: "#2563eb",
    color: "#fff",
    cursor: "pointer",
  },
  sessionList: {
    listStyle: "none",
    display: "flex",
    flexDirection: "column",
    gap: "0.25rem",
    overflowY: "auto",
    flex: 1,
  },
  sessionItem: {
    padding: "0.5rem",
    borderRadius: "0.375rem",
    cursor: "pointer",
  },
  meta: {
    fontSize: "0.75rem",
    color: "#94a3b8",
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
