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

  // Workspace state: a list of local directories the agent can work in.
  const [workspaces, setWorkspaces] = useState<api.Workspace[]>([]);
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);
  const [showAddWorkspace, setShowAddWorkspace] = useState(false);
  const [newWsName, setNewWsName] = useState("");
  const [newWsPath, setNewWsPath] = useState("");
  const [newWsTools, setNewWsTools] = useState<string[]>([]);

  // Every tool the agent can call is selectable per-workspace. This is a hard
  // ceiling: a workspace restricts the agent to the checked tools, and the
  // per-request chat tool menu (and web_search toggle) narrows further from there.
  const selectableWorkspaceTools = tools;

  useEffect(() => {
    loadSessions();
    loadWorkspaces();
    api.listProviders().then((ps) => {
      setProviders(ps);
      if (ps.length) setProvider(ps[0]);
    }).catch(console.error);
    api.listTools().then(setTools).catch(console.error);
  }, []);

  const loadSessions = () => {
    api.listSessions().then(setSessions).catch(console.error);
  };

  const loadWorkspaces = () => {
    api.listWorkspaces().then(setWorkspaces).catch(console.error);
  };

  const createSession = async () => {
    const t = title.trim() || "New chat";
    const s = await api.createSession({
      title: t,
      provider,
      model,
      workspace_id: activeWorkspaceId ?? undefined,
    });
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

  const applyPickedDirectory = (picked: string | null) => {
    if (!picked) return;
    setNewWsPath(picked);
    const base = picked.split("/").filter(Boolean).pop() || picked;
    if (!newWsName.trim()) setNewWsName(base);
  };

  const selectWorkspaceDirectory = async () => {
    applyPickedDirectory(await api.pickDirectory());
  };

  const openAddWorkspace = async () => {
    setShowAddWorkspace(true);
    setNewWsName("");
    setNewWsPath("");
    setNewWsTools(selectableWorkspaceTools.map((t) => t.name));
    // In the desktop app, immediately open the native directory picker so the
    // user selects a directory rather than typing a path. In a plain browser
    // this resolves to null and the path is entered manually instead.
    applyPickedDirectory(await api.pickDirectory());
  };

  const toggleWorkspaceTool = (name: string) => {
    setNewWsTools((prev) =>
      prev.includes(name) ? prev.filter((t) => t !== name) : [...prev, name],
    );
  };

  const addWorkspace = async () => {
    const path = newWsPath.trim();
    if (!path) return;
    try {
      await api.createWorkspace({
        name: newWsName.trim(),
        path,
        tools: newWsTools,
      });
      setNewWsName("");
      setNewWsPath("");
      setNewWsTools([]);
      setShowAddWorkspace(false);
      loadWorkspaces();
    } catch (e) {
      console.error(e);
    }
  };

  const cancelAddWorkspace = () => {
    setShowAddWorkspace(false);
    setNewWsName("");
    setNewWsPath("");
    setNewWsTools([]);
  };

  const removeWorkspace = async (w: api.Workspace) => {
    if (!window.confirm(`Delete workspace "${w.name}"?`)) return;
    try {
      await api.deleteWorkspace(w.id);
      if (activeWorkspaceId === w.id) setActiveWorkspaceId(null);
      loadWorkspaces();
    } catch (e) {
      console.error(e);
    }
  };

  const changeSessionWorkspace = async (id: string, workspaceId: string) => {
    try {
      await api.updateSession(id, { workspace_id: workspaceId });
      loadSessions();
    } catch (e) {
      console.error(e);
    }
  };

  const currentSession = sessions.find((s) => s.id === currentId);
  const activeWorkspace = workspaces.find((w) => w.id === activeWorkspaceId);
  const currentWorkspace = workspaces.find(
    (w) => w.id === currentSession?.workspace_id,
  );

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
          <div style={styles.hint}>
            工作区: {activeWorkspace ? activeWorkspace.name : "无"}
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

        <div style={styles.workspaceSection}>
          <div style={styles.workspaceHeader}>
            <strong>工作区</strong>
            <button
              style={styles.addWorkspace}
              onClick={openAddWorkspace}
            >
              ＋ 添加
            </button>
          </div>
          {showAddWorkspace && (
            <div style={styles.newWorkspaceForm}>
              <input
                style={styles.input}
                placeholder="名称 (可选)"
                value={newWsName}
                onChange={(e) => setNewWsName(e.target.value)}
              />
              <div style={styles.row}>
                <input
                  style={{ ...styles.input, flex: 1 }}
                  placeholder="目录路径 /path/to/dir"
                  value={newWsPath}
                  onChange={(e) => setNewWsPath(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") addWorkspace();
                  }}
                />
                {api.isTauri() && (
                  <button
                    style={styles.addWorkspace}
                    title="选择目录"
                    onClick={selectWorkspaceDirectory}
                  >
                    选择目录
                  </button>
                )}
              </div>
              {!api.isTauri() && (
                <div style={styles.hint}>浏览器环境不支持原生目录选择，请手动填写目录路径。</div>
              )}
              <div style={styles.toolChecks}>
                <div style={styles.toolChecksLabel}>允许使用的工具</div>
                {selectableWorkspaceTools.map((t) => (
                  <label key={t.name} style={styles.toolCheck}>
                    <input
                      type="checkbox"
                      checked={newWsTools.includes(t.name)}
                      onChange={() => toggleWorkspaceTool(t.name)}
                    />
                    <span>{t.name}</span>
                  </label>
                ))}
              </div>
              <div style={styles.row}>
                <button style={styles.newChat} onClick={addWorkspace}>添加</button>
                <button style={styles.cancelButton} onClick={cancelAddWorkspace}>取消</button>
              </div>
            </div>
          )}
          <ul style={styles.workspaceList}>
            {workspaces.map((w) => (
              <li
                key={w.id}
                style={{
                  ...styles.workspaceItem,
                  background: w.id === activeWorkspaceId ? "#334155" : "transparent",
                }}
                onClick={() => setActiveWorkspaceId(w.id)}
              >
                <div style={styles.workspaceRow}>
                  <span style={styles.workspaceName}>{w.name}</span>
                  <button
                    style={styles.action}
                    title="删除工作区"
                    onClick={(e) => {
                      e.stopPropagation();
                      removeWorkspace(w);
                    }}
                  >
                    🗑
                  </button>
                </div>
                <div style={styles.workspacePath}>{w.path}</div>
              </li>
            ))}
          </ul>
        </div>

        <div style={styles.tools}>
          <strong>Tools ({tools.length})</strong>
        </div>
      </aside>
      <main style={styles.main}>
        {currentId ? (
          <>
            <div style={styles.chatHeader}>
              <span style={styles.chatHeaderTitle}>{currentSession?.title ?? ""}</span>
              <select
                style={styles.select}
                value={currentSession?.workspace_id ?? ""}
                onChange={(e) => changeSessionWorkspace(currentId, e.target.value)}
              >
                <option value="">无工作区</option>
                {workspaces.map((w) => (
                  <option key={w.id} value={w.id}>{w.name}</option>
                ))}
              </select>
              {currentWorkspace && (
                <span style={styles.chatHeaderPath}>{currentWorkspace.path}</span>
              )}
            </div>
            <div style={styles.chatBody}>
              <Chat sessionId={currentId} tools={tools} />
            </div>
          </>
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
    minWidth: "1600px",
    maxWidth: "3300px",
    margin: "0 auto",
  },
  sidebar: {
    width: "280px",
    flexShrink: 0,
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
  cancelButton: {
    padding: "0.5rem 0.75rem",
    borderRadius: "0.375rem",
    border: "1px solid #334155",
    background: "transparent",
    color: "#cbd5e1",
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
  hint: {
    fontSize: "0.75rem",
    color: "#94a3b8",
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
    minHeight: 0,
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
  workspaceSection: {
    display: "flex",
    flexDirection: "column",
    gap: "0.5rem",
    borderTop: "1px solid #334155",
    paddingTop: "0.75rem",
    flexShrink: 0,
  },
  workspaceHeader: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    color: "#cbd5e1",
    fontSize: "0.875rem",
  },
  addWorkspace: {
    padding: "0.25rem 0.5rem",
    borderRadius: "0.375rem",
    border: "1px solid #334155",
    background: "#0f172a",
    color: "#cbd5e1",
    cursor: "pointer",
    fontSize: "0.8rem",
  },
  newWorkspaceForm: {
    display: "flex",
    flexDirection: "column",
    gap: "0.4rem",
  },
  toolChecks: {
    display: "flex",
    flexDirection: "column",
    gap: "0.2rem",
    maxHeight: "120px",
    overflowY: "auto",
  },
  toolChecksLabel: {
    fontSize: "0.72rem",
    color: "#94a3b8",
  },
  toolCheck: {
    display: "flex",
    alignItems: "center",
    gap: "0.35rem",
    fontSize: "0.78rem",
    color: "#cbd5e1",
    cursor: "pointer",
  },
  workspaceList: {
    listStyle: "none",
    display: "flex",
    flexDirection: "column",
    gap: "0.25rem",
    overflowY: "auto",
    maxHeight: "160px",
    margin: 0,
    padding: 0,
  },
  workspaceItem: {
    padding: "0.4rem 0.5rem",
    borderRadius: "0.375rem",
    cursor: "pointer",
  },
  workspaceRow: {
    display: "flex",
    alignItems: "center",
    gap: "0.25rem",
  },
  workspaceName: {
    flex: 1,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
    fontSize: "0.85rem",
    color: "#e2e8f0",
  },
  workspacePath: {
    fontSize: "0.72rem",
    color: "#94a3b8",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
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
    minWidth: 0,
    overflow: "hidden",
    gap: "0.75rem",
  },
  chatHeader: {
    display: "flex",
    alignItems: "center",
    gap: "0.75rem",
    flexShrink: 0,
  },
  chatHeaderTitle: {
    fontWeight: 600,
    color: "#e2e8f0",
    flex: 1,
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  chatHeaderPath: {
    fontSize: "0.8rem",
    color: "#94a3b8",
    maxWidth: "40%",
    overflow: "hidden",
    textOverflow: "ellipsis",
    whiteSpace: "nowrap",
  },
  chatBody: {
    flex: 1,
    minHeight: 0,
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
