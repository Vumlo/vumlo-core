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

/** Text for an error box: the first field message, else the detail, else the title. */
export const errorText = (e: ApiError | undefined): string | undefined => e && (e.errors[0]?.message ?? e.detail ?? e.title);

let sessionExpired: (() => void) | undefined;
/** The shell listens: a 401 on any call but sign-in means the session ended, and `/me` is asked again to confirm it. */
export function onSessionExpired(listener: () => void) {
  sessionExpired = listener;
  return () => {
    if (sessionExpired === listener) sessionExpired = undefined;
  };
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

/** The answer of a call as the caller wants it: the body, nothing for a 204, `{queued}` for a write kept for later, else an ApiError. */
function settle<T>(res: Reply, path: string): T {
  if (res.status === 204) return undefined as T;
  if (res.queued && res.json === undefined) return { queued: true } as T;
  if (res.status < 200 || res.status > 299) {
    if (res.status === 401 && !path.startsWith("/auth/") && path !== "/me") sessionExpired?.();
    const p = (res.json ?? {}) as { title?: string; detail?: string; errors?: FieldError[] };
    throw new ApiError(res.status, p.title ?? res.statusText, p.detail, p.errors ?? []);
  }
  return res.json as T;
}

async function request<T>(method: Method, path: string, body?: unknown, options?: Options): Promise<T> {
  return settle<T>(await (isDesktop() ? viaDesktop : viaFetch)(method, path, body, options), path);
}

/**
 * A request whose body is the raw bytes of a file (`POST /files`). The browser sends them as they are; the desktop
 * app has one command for it, because its `api` command only carries JSON.
 */
export async function requestRaw<T>(method: "POST", path: string, body: Blob | ArrayBuffer, headers: Record<string, string>): Promise<T> {
  if (!isDesktop()) {
    const res = await fetch(`/api/v1${path}`, { method, credentials: "same-origin", headers, body });
    const text = res.status === 204 ? "" : await res.text();
    return settle<T>({ status: res.status, statusText: res.statusText, json: text ? JSON.parse(text) : undefined }, path);
  }
  const url = new URL(path, "http://x");
  if (url.pathname !== "/files") throw new Error(`${method} ${url.pathname} has no raw form in the desktop app`);
  const { invoke } = await import("@tauri-apps/api/core");
  const bytes = new Uint8Array(body instanceof Blob ? await body.arrayBuffer() : body);
  const target = url.searchParams.get("for") ?? "";
  const name = url.searchParams.get("name") ?? "";
  const mime = headers["Content-Type"] ?? "application/octet-stream";
  // The bytes are the raw body of the call and the rest travels in headers (Tauri's binary IPC, which sets `Content-Type`
  // itself, so the file's type has a header of its own). The command answers
  // with the FileRef, or fails with "<status>: <detail>", the server's refusal.
  return invoke<T>("upload_file", bytes, { headers: { "x-vumlo-target": target, "x-vumlo-name": encodeURIComponent(name), "x-vumlo-mime": mime } }).catch((e: unknown) => {
    const refused = typeof e === "string" ? /^(\d{3}): ([\s\S]*)$/.exec(e) : null;
    throw refused ? new ApiError(Number(refused[1]), "upload", refused[2] || undefined) : new ApiError(0, "upload", typeof e === "string" ? e : undefined);
  });
}

/** At most `maxBytes` of the start of a file, as text. `url` is the file's address (`fileUrl` of lib/files). */
export async function readText(url: string, maxBytes: number): Promise<string> {
  const res = await fetch(url, { credentials: "same-origin", headers: { Range: `bytes=0-${maxBytes - 1}` } });
  if (!res.ok || !res.body) throw new ApiError(res.status, res.statusText);
  const reader = res.body.getReader();
  const chunks: Uint8Array[] = [];
  let got = 0;
  while (got < maxBytes) {
    const { done, value } = await reader.read();
    if (done) break;
    chunks.push(value);
    got += value.length;
  }
  void reader.cancel();
  const all = new Uint8Array(got);
  let at = 0;
  for (const c of chunks) {
    all.set(c, at);
    at += c.length;
  }
  return new TextDecoder().decode(all.subarray(0, maxBytes));
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
