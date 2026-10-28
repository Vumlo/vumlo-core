"use client";

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { ApiError } from "./api/client";
import { translate } from "./i18n/core";

type Entry = { data?: unknown; error?: ApiError; loading: boolean; stale: boolean; at?: number };

const EMPTY: Entry = { loading: false, stale: true };
const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();
/** Bumped by `clearQueries`, so an answer that was in flight for the previous person is dropped. */
let generation = 0;
/** How long an answer is trusted when a screen opens or the tab comes back. Real time, not `lib/clock`: it ages a cache, it is not shown. */
const MAX_AGE_MS = 30_000;

function emit() {
  listeners.forEach((l) => l());
}

function put(key: string, entry: Entry) {
  entries.set(key, entry);
  emit();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Marks every cached query whose key starts with `prefix` stale; mounted users refetch. */
export function invalidate(prefix: string) {
  for (const [key, entry] of entries) {
    if (key.startsWith(prefix)) entries.set(key, { ...entry, stale: true });
  }
  emit();
}

export function clearQueries() {
  generation += 1;
  entries.clear();
  emit();
}

/** Marks what failed, or is older than `MAX_AGE_MS`, stale. Returns true when something changed. */
function ageOut(only?: string): boolean {
  let changed = false;
  const old = (e: Entry) => !e.loading && !e.stale && (e.error !== undefined || Date.now() - (e.at ?? 0) > MAX_AGE_MS);
  for (const [key, entry] of entries) {
    if ((only === undefined || key === only) && old(entry)) {
      entries.set(key, { ...entry, stale: true });
      changed = true;
    }
  }
  return changed;
}

/** Loads again every query that failed ("Try again"). */
export function retryFailed() {
  for (const [key, entry] of entries) if (entry.error) entries.set(key, { ...entry, stale: true });
  emit();
}

let waking = false;
/** A tab that comes back, or a connection that returns, refreshes what is old or failed. */
function wakeOnReturn() {
  if (waking || typeof window === "undefined") return;
  waking = true;
  const wake = () => {
    if (document.visibilityState === "visible" && ageOut()) emit();
  };
  document.addEventListener("visibilitychange", wake);
  window.addEventListener("online", wake);
}

export type Query<T> = {
  data: T | undefined;
  error: ApiError | undefined;
  isLoading: boolean;
};

/**
 * Reads `key` from the cache and fetches it when missing or stale. The old data stays
 * visible while a stale entry reloads. A null key disables the query.
 */
export function useQuery<T>(key: string | null, fetcher: () => Promise<T>): Query<T> {
  const fetcherRef = useRef(fetcher);
  useEffect(() => {
    fetcherRef.current = fetcher;
  });
  const entry = useSyncExternalStore(
    subscribe,
    () => (key ? (entries.get(key) ?? EMPTY) : EMPTY),
    () => EMPTY,
  );
  const needsLoad = key !== null && entry.stale && !entry.loading;
  // A screen that opens shows what it has and asks again when that is old or failed.
  useEffect(() => {
    wakeOnReturn();
    if (key && ageOut(key)) emit();
  }, [key]);
  useEffect(() => {
    if (!key || !needsLoad) return;
    const previous = entries.get(key);
    const mine = generation;
    put(key, { ...previous, loading: true, stale: false });
    // An `invalidate` that came while this was in flight leaves `stale` set: the answer is kept, then asked for again.
    const settle = (next: Entry) => {
      if (mine !== generation) return;
      put(key, { ...next, loading: false, stale: entries.get(key)?.stale === true });
    };
    fetcherRef.current().then(
      (data) => settle({ data, at: Date.now(), loading: false, stale: false }),
      (error: unknown) =>
        settle({
          data: previous?.data,
          at: previous?.at,
          error: error instanceof ApiError ? error : new ApiError(0, translate("error.network")),
          loading: false,
          stale: false,
        }),
    );
  }, [key, needsLoad]);
  return {
    data: entry.data as T | undefined,
    error: entry.error,
    isLoading: key !== null && entry.data === undefined && !entry.error,
  };
}

export type Mutation<A extends unknown[], R> = {
  run: (...args: A) => Promise<R | undefined>;
  pending: boolean;
  error: ApiError | undefined;
  reset: () => void;
};

/** Runs a write. `run` resolves to undefined when it failed; the error is in `error`. */
export function useMutation<A extends unknown[], R>(fn: (...args: A) => Promise<R>): Mutation<A, R> {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<ApiError | undefined>();
  const fnRef = useRef(fn);
  useEffect(() => {
    fnRef.current = fn;
  });
  const run = useCallback(async (...args: A) => {
    setPending(true);
    setError(undefined);
    try {
      return await fnRef.current(...args);
    } catch (e) {
      setError(e instanceof ApiError ? e : new ApiError(0, translate("error.network")));
      return undefined;
    } finally {
      setPending(false);
    }
  }, []);
  const reset = useCallback(() => setError(undefined), []);
  return { run, pending, error, reset };
}
