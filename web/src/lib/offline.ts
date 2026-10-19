"use client";

import { useSyncExternalStore } from "react";
import { api, isDesktop } from "./api/client";
import { fetchAll } from "./api/hooks/util";
import { invalidate } from "./query";

// The desktop app tells the window about the connection and the send queue (ADR-0060). In a browser there
// is nothing to tell: `useSync()` is null and every screen behaves as before.

export type SyncState = {
  online: boolean;
  pending: number;
  failed: number;
  paused: boolean;
  /** Unix milliseconds of the last answer from the server. */
  last_ok: number | null;
  sent: number;
  /** The paths of the writes that wait. */
  queued: string[];
};

export type OutboxItem = {
  id: number;
  method: string;
  path: string;
  body: string | null;
  state: "pending" | "sending" | "failed";
  status: number | null;
  /** The server's Problem Details body, as text. */
  problem: string | null;
  attempts: number;
  created_at: number;
};

let state: SyncState | null = null;
const listeners = new Set<() => void>();
let started: Promise<void> | null = null;

function set(next: SyncState) {
  state = next;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** Starts listening to the app. Safe to call again; does nothing outside the desktop app. */
export function startSync() {
  if (!isDesktop()) return Promise.resolve();
  started ??= (async () => {
    const [{ invoke }, { listen }] = await Promise.all([import("@tauri-apps/api/core"), import("@tauri-apps/api/event")]);
    set(await invoke<SyncState>("sync_state"));
    await listen<SyncState>("sync", (event) => {
      const before = state;
      set(event.payload);
      // The queue sent something: what the screens hold is older than the server's.
      if (before && event.payload.sent > before.sent) invalidate("/");
    });
    window.addEventListener("online", () => void invoke("sync_now"));
  })();
  return started;
}

/** The state of the connection and the queue, or null in a browser. */
export function useSync(): SyncState | null {
  return useSyncExternalStore(subscribe, () => state, () => null);
}

/** True while a write to `path` waits to be sent. */
export function useQueued(path: string) {
  return useSync()?.queued.includes(path) === true;
}

/** True in the desktop app while the server cannot be reached. */
export function useOffline() {
  const sync = useSync();
  return sync !== null && !sync.online;
}

/** Tells the app the language of the window, for its notifications and its tray. */
export async function syncLanguage(language: string) {
  if (!isDesktop()) return;
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("set_language", { language }).catch(() => undefined);
}

/**
 * Calls `go` with the screen a link from outside asked for (`vumlo-<abbr>://...`), now if the app was opened with
 * one and again whenever one arrives. Returns the function that stops listening.
 */
export function onNavigate(go: (route: string) => void) {
  if (!isDesktop()) return () => undefined;
  let stop: (() => void) | undefined;
  let stopped = false;
  void (async () => {
    const [{ invoke }, { listen }] = await Promise.all([import("@tauri-apps/api/core"), import("@tauri-apps/api/event")]);
    const take = async () => {
      const route = await invoke<string | null>("take_route");
      if (route) go(route);
    };
    await take();
    const off = await listen("navigate", () => void take());
    if (stopped) off();
    else stop = off;
  })();
  return () => {
    stopped = true;
    stop?.();
  };
}

/** How a sign-in with Google that the app started ended (ADR-0062). */
export type GoogleOutcome = { ok: true; user: import("./session").User } | { ok: false; error: string };

/** Opens Google's page in the system browser; the answer comes as the `google` event. */
export async function startGoogleInApp(loginHint: string) {
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("google_start", { loginHint: loginHint || null });
}

/** Calls `done` when a sign-in with Google ends in the app. Returns the function that stops listening. */
export function onGoogleOutcome(done: (outcome: GoogleOutcome) => void) {
  if (!isDesktop()) return () => undefined;
  let stop: (() => void) | undefined;
  let stopped = false;
  void import("@tauri-apps/api/event").then(({ listen }) =>
    listen<GoogleOutcome>("google", (event) => done(event.payload)).then((off) => {
      if (stopped) off();
      else stop = off;
    }),
  );
  return () => {
    stopped = true;
    stop?.();
  };
}

export async function syncNow() {
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("sync_now");
}

export async function outboxList() {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<OutboxItem[]>("outbox_list");
}

export async function outboxDiscard(id: number) {
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("outbox_discard", { id });
}

// --- keeping the copy warm ---------------------------------------------------------------------

type Tree = { modules?: { items?: { id: string; kind: string }[]; submodules?: { items?: { id: string; kind: string }[] }[] }[] };

const CONCURRENCY = 4;

/** Runs `jobs` with at most `CONCURRENCY` at a time; a failed one does not stop the others. */
async function pool(jobs: (() => Promise<unknown>)[]) {
  const queue = [...jobs];
  await Promise.all(
    Array.from({ length: CONCURRENCY }, async () => {
      for (let job = queue.shift(); job; job = queue.shift()) await job().catch(() => undefined);
    }),
  );
}

let prefetching = false;

/**
 * Reads what a person is likely to want without a connection, so the app holds it (every read goes through
 * the app's cache, ADR-0060). The paths are exactly those the screens use. A student gets their grades and
 * submissions too; a teacher gets the people of a course.
 */
export async function prefetch(role: string) {
  if (!isDesktop() || prefetching) return;
  prefetching = true;
  try {
    const student = role === "student";
    const courses = await fetchAll<{ id: string }>("/me/courses").catch(() => []);
    const jobs: (() => Promise<unknown>)[] = [
      () => api.get("/me/course-cards"),
      () => api.get("/me/work"),
      () => api.get("/editor/profiles"),
      ...(student ? [] : [() => api.get("/me/teaching")]),
    ];
    for (const { id } of courses) {
      jobs.push(
        () => api.get(`/courses/${id}`),
        () => api.get(`/courses/${id}/info`),
        () => api.get(`/courses/${id}/item-meta`),
        () => api.get(`/courses/${id}/people`),
        ...(student ? [() => api.get(`/courses/${id}/my-grades`)] : []),
        async () => {
          const tree = await api.get<Tree>(`/courses/${id}/content`);
          const items = (tree.modules ?? []).flatMap((m) => [...(m.items ?? []), ...(m.submodules ?? []).flatMap((s) => s.items ?? [])]);
          await pool(
            items.flatMap((item) => [
              () => api.get(`/items/${item.id}`),
              ...(student && item.kind === "assignment" ? [() => api.get(`/assignments/${item.id}/submission`)] : []),
            ]),
          );
        },
      );
    }
    await pool(jobs);
  } finally {
    prefetching = false;
  }
}
