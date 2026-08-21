export interface Session {
  id: string;
  title: string;
  provider: string;
  model: string;
  messages: Message[];
  pinned?: boolean;
  created_at: string;
  updated_at: string;
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

export async function fetchJSON<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, {
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

export async function createSession(payload: { title: string; provider: string; model: string }): Promise<Session> {
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
  patch: { title?: string; pinned?: boolean },
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
): Promise<{ message?: Message }> {
  return fetchJSON<{ message?: Message }>(`/api/v1/sessions/${id}/chat`, {
    method: "POST",
    body: JSON.stringify({
      content,
      tool_names: opts.toolNames ?? [],
      web_search: opts.webSearch ?? false,
      attachment_ids: opts.attachmentIds ?? [],
    }),
  });
}

export async function uploadFile(file: File): Promise<Attachment> {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch("/api/v1/uploads", {
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
