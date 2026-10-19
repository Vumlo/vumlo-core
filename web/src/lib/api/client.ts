/** One field error from a 422 Problem Details body (ADR-0018). */
export type FieldError = { field: string; message: string };

export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly title: string,
    readonly detail?: string,
    readonly errors: FieldError[] = [],
  ) {
    super(detail ?? title);
  }
}

type Method = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

/** True inside the desktop app (ADR-0056). There the requests are made by Rust, with the token it keeps. */
export const isDesktop = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

type Reply = { status: number; statusText: string; json: unknown };

// The web app: the API is on the same site, so the address is relative and the session is a cookie (ADR-0013).
async function viaFetch(method: Method, path: string, body: unknown): Promise<Reply> {
  const res = await fetch(`/api/v1${path}`, {
    method,
    credentials: "same-origin",
    headers: body === undefined ? undefined : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = res.status === 204 ? "" : await res.text();
  return { status: res.status, statusText: res.statusText, json: text ? JSON.parse(text) : undefined };
}

// The desktop app: the `api` command of desktop/ sends it to the compiled-in address (ADR-0056).
async function viaDesktop(method: Method, path: string, body: unknown): Promise<Reply> {
  const { invoke } = await import("@tauri-apps/api/core");
  const r = await invoke<{ status: number; body: unknown }>("api", { method, path, body: body ?? null });
  return { status: r.status, statusText: "", json: r.body ?? undefined };
}

async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  const res = await (isDesktop() ? viaDesktop : viaFetch)(method, path, body);
  if (res.status === 204) return undefined as T;
  if (res.status < 200 || res.status > 299) {
    const p = (res.json ?? {}) as { title?: string; detail?: string; errors?: FieldError[] };
    throw new ApiError(res.status, p.title ?? res.statusText, p.detail, p.errors ?? []);
  }
  return res.json as T;
}

export const api = {
  get: <T>(path: string) => request<T>("GET", path),
  post: <T>(path: string, body?: unknown) => request<T>("POST", path, body ?? {}),
  put: <T>(path: string, body?: unknown) => request<T>("PUT", path, body ?? {}),
  patch: <T>(path: string, body?: unknown) => request<T>("PATCH", path, body ?? {}),
  delete: <T>(path: string) => request<T>("DELETE", path),
};

/** A list endpoint answers `{items, next_cursor}` (ADR-0018). */
export type Page<T> = { items: T[]; next_cursor: string | null };
