export interface Session {
  id: string;
  title: string;
  provider: string;
  model: string;
  messages: Message[];
  created_at: string;
  updated_at: string;
}

export interface Message {
  role: string;
  content: string;
  sent_at?: string;
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

export async function chat(id: string, content: string): Promise<{ message?: Message }> {
  return fetchJSON<{ message?: Message }>(`/api/v1/sessions/${id}/chat`, {
    method: "POST",
    body: JSON.stringify({ content }),
  });
}

export async function listProviders(): Promise<string[]> {
  const data = await fetchJSON<{ providers: string[] }>("/api/v1/providers");
  return data.providers ?? [];
}

export async function listTools(): Promise<unknown[]> {
  const data = await fetchJSON<{ tools: unknown[] }>("/api/v1/tools");
  return data.tools ?? [];
}
