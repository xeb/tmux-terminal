import { getApiBaseUrl } from './config';

export interface TmuxWindow {
  target: string;
  name: string;
}

export interface ApiResponse {
  success: boolean;
  error?: string;
}

async function request<T>(
  path: string,
  options: RequestInit = {},
  timeoutMs = 8000,
): Promise<T> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const res = await fetch(`${getApiBaseUrl()}${path}`, {
      ...options,
      signal: controller.signal,
    });
    clearTimeout(timer);
    if (!res.ok) {
      const text = await res.text().catch(() => 'Unknown error');
      throw new Error(`HTTP ${res.status}: ${text}`);
    }
    return res.json() as Promise<T>;
  } catch (e) {
    clearTimeout(timer);
    throw e;
  }
}

export async function listWindows(): Promise<TmuxWindow[]> {
  return request<TmuxWindow[]>('/api/windows');
}

export async function capturePane(target: string): Promise<string> {
  const data = await request<{ content: string }>('/api/capture', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ target }),
  });
  return data.content ?? '';
}

export async function sendCommand(
  command: string,
  session: string,
): Promise<ApiResponse> {
  return request<ApiResponse>('/api/send', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ command, session }),
  });
}

export async function sendKey(
  key: string,
  session: string,
): Promise<ApiResponse> {
  return request<ApiResponse>('/api/send-key', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key, session }),
  });
}

export async function createWindow(): Promise<
  ApiResponse & { target?: string }
> {
  return request('/api/new-window', { method: 'POST' });
}

export async function renameWindow(
  target: string,
  name: string,
): Promise<ApiResponse> {
  return request<ApiResponse>('/api/rename-window', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ target, name }),
  });
}

export interface BugReportPayload {
  description?: string;
  app_version?: string;
  build_date?: string;
  device?: string;
  os?: string;
  screenshots?: Array<{ name: string; data: string }>;
}

export async function submitBugReport(
  payload: BugReportPayload,
): Promise<{ success: boolean; id?: string; error?: string }> {
  return request('/api/bug-report', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  }, 30000); // 30s timeout for image upload
}
