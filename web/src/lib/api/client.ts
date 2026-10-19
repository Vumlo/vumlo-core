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

/** What `post` resolves to when the write was queued and has no answer yet (the desktop app, ADR-0060). */
export type Queued = { queued: true };
export const isQueued = (value: unknown): value is Queued => typeof value === "object" && value !== null && (value as Queued).queued === true;

/** `queue`: on the desktop, keep the write until there is a connection instead of failing. Elsewhere it does nothing. */
export type Options = { queue?: boolean };

type Reply = { status: number; statusText: string; json: unknown; queued?: boolean };

// The web app: the API is on the same site, so the address is relative and the session is a cookie (ADR-0013).
async function viaFetch(method: Method, path: string, body: unknown, _options?: Options): Promise<Reply> {
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
async function viaDesktop(method: Method, path: string, body: unknown, options?: Options): Promise<Reply> {
  const { invoke } = await import("@tauri-apps/api/core");
  const queue = options?.queue === true;
  // One key per action, kept by the app for every retry of it (ADR-0058).
  const r = await invoke<{ status: number; body: unknown; queued: boolean }>("api", { method, path, body: body ?? null, queue, idemKey: queue ? crypto.randomUUID() : null }).catch((e: unknown) => {
    // The app could not keep the session in the system's keychain: say so, not "network error".
    if (typeof e === "string" && e.startsWith("keychain:")) throw new ApiError(0, "keychain", e.slice("keychain:".length).trim());
    throw e;
  });
  return { status: r.status, statusText: "", json: r.body ?? undefined, queued: r.queued };
}

async function request<T>(method: Method, path: string, body?: unknown, options?: Options): Promise<T> {
  const res = await (isDesktop() ? viaDesktop : viaFetch)(method, path, body, options);
  if (res.status === 204) return undefined as T;
  if (res.queued && res.json === undefined) return { queued: true } as T;
  if (res.status < 200 || res.status > 299) {
    const p = (res.json ?? {}) as { title?: string; detail?: string; errors?: FieldError[] };
    throw new ApiError(res.status, p.title ?? res.statusText, p.detail, p.errors ?? []);
  }
  return res.json as T;
}

export const api = {
  get: <T>(path: string) => request<T>("GET", path),
  post: <T>(path: string, body?: unknown, options?: Options) => request<T>("POST", path, body ?? {}, options),
  put: <T>(path: string, body?: unknown, options?: Options) => request<T>("PUT", path, body ?? {}, options),
  patch: <T>(path: string, body?: unknown) => request<T>("PATCH", path, body ?? {}),
  delete: <T>(path: string) => request<T>("DELETE", path),
};

/** A list endpoint answers `{items, next_cursor}` (ADR-0018). */
export type Page<T> = { items: T[]; next_cursor: string | null };
