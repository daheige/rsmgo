export interface Session {
  id: string;
  title: string;
  provider: string;
  model: string;
  messages: Message[];
  pinned?: boolean;
  workspace_id?: string;
  created_at: string;
  updated_at: string;
}

export interface Workspace {
  id: string;
  name: string;
  path: string;
  tools?: string[];
  created_at: string;
}

export interface Message {
  role: string;
  content: string;
  sent_at?: string;
}

export interface ToolInfo {
  name: string;
  description: string;
  parameters_schema: string;
}

export interface Attachment {
  id: string;
  name: string;
  content_type: string;
  size: number;
}

export interface ChatOptions {
  toolNames?: string[];
  webSearch?: boolean;
  attachmentIds?: string[];
}

// Resolve the control-plane base URL.
//
// - Desktop (Tauri) static build: no Node server runs inside the WebView, so the
//   build injects NEXT_PUBLIC_RSMGO_CONTROL_URL to point at the control plane.
// - Web development: talk to the control plane directly to avoid Next.js dev
//   proxy timeout/ECONNRESET issues on long chat requests.
// - Web production: served standalone and requests stay same-origin via rewrites.
function baseUrl(): string {
  const configured = process.env.NEXT_PUBLIC_RSMGO_CONTROL_URL;
  if (configured) return configured;
  if (typeof window !== "undefined" && process.env.NODE_ENV === "development") {
    return "http://localhost:9090";
  }
  return "";
}

export async function fetchJSON<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${baseUrl()}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...init,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`HTTP ${res.status}: ${text}`);
  }
  return res.json();
}

export async function listSessions(): Promise<Session[]> {
  const data = await fetchJSON<{ sessions: Session[] }>("/api/v1/sessions");
  return data.sessions ?? [];
}

export async function createSession(payload: {
  title: string;
  provider: string;
  model: string;
  workspace_id?: string;
}): Promise<Session> {
  return fetchJSON<Session>("/api/v1/sessions", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export async function getSession(id: string): Promise<Session> {
  return fetchJSON<Session>(`/api/v1/sessions/${id}`);
}

export async function updateSession(
  id: string,
  patch: { title?: string; pinned?: boolean; workspace_id?: string },
): Promise<Session> {
  return fetchJSON<Session>(`/api/v1/sessions/${id}`, {
    method: "PATCH",
    body: JSON.stringify(patch),
  });
}

export async function deleteSession(id: string): Promise<void> {
  await fetchJSON<{ deleted: boolean }>(`/api/v1/sessions/${id}`, {
    method: "DELETE",
  });
}

export async function chat(
  id: string,
  content: string,
  opts: ChatOptions = {},
  signal?: AbortSignal,
): Promise<{ message?: Message }> {
  return fetchJSON<{ message?: Message }>(`/api/v1/sessions/${id}/chat`, {
    method: "POST",
    body: JSON.stringify({
      content,
      tool_names: opts.toolNames ?? [],
      web_search: opts.webSearch ?? false,
      attachment_ids: opts.attachmentIds ?? [],
    }),
    signal,
  });
}

export async function cancelChat(id: string): Promise<void> {
  await fetchJSON<{ cancelled: boolean }>(`/api/v1/sessions/${id}/chat/cancel`, {
    method: "POST",
  });
}

export async function listWorkspaces(): Promise<Workspace[]> {
  const data = await fetchJSON<{ workspaces: Workspace[] }>("/api/v1/workspaces");
  return data.workspaces ?? [];
}

export async function createWorkspace(payload: {
  name: string;
  path: string;
  tools?: string[];
}): Promise<Workspace> {
  return fetchJSON<Workspace>("/api/v1/workspaces", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

export async function deleteWorkspace(id: string): Promise<void> {
  await fetchJSON<{ deleted: boolean }>(`/api/v1/workspaces/${id}`, {
    method: "DELETE",
  });
}

// Whether the app is running inside the Tauri desktop WebView, where the native
// directory picker is available.
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

// Open a native directory picker (Tauri desktop only) and return the selected
// directory's absolute path. Returns null when running in a plain browser (no
// native picker) or when the user cancels the dialog.
export async function pickDirectory(): Promise<string | null> {
  if (!isTauri()) return null;
  const w = window as unknown as {
    __TAURI_INTERNALS__?: {
      invoke: (cmd: string, args?: unknown) => Promise<unknown>;
    };
  };
  if (!w.__TAURI_INTERNALS__?.invoke) return null;
  const result = await w.__TAURI_INTERNALS__.invoke("pick_directory");
  return typeof result === "string" && result ? result : null;
}

export async function uploadFile(file: File): Promise<Attachment> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`${baseUrl()}/api/v1/uploads`, {
    method: "POST",
    body: form,
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`HTTP ${res.status}: ${text}`);
  }
  return res.json();
}

export function attachmentUrl(id: string): string {
  return `/api/v1/uploads/${id}`;
}

export async function listProviders(): Promise<string[]> {
  const data = await fetchJSON<{ providers: string[] }>("/api/v1/providers");
  return data.providers ?? [];
}

export async function listTools(): Promise<ToolInfo[]> {
  const data = await fetchJSON<{ tools: ToolInfo[] }>("/api/v1/tools");
  return data.tools ?? [];
}
