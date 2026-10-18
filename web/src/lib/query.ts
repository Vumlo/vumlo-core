"use client";

import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { ApiError } from "./api/client";
import { translate } from "./i18n/core";

type Entry = { data?: unknown; error?: ApiError; loading: boolean; stale: boolean };

const EMPTY: Entry = { loading: false, stale: true };
const entries = new Map<string, Entry>();
const listeners = new Set<() => void>();

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
  entries.clear();
  emit();
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
  useEffect(() => {
    if (!key || !needsLoad) return;
    const previous = entries.get(key);
    put(key, { ...previous, loading: true, stale: false });
    fetcherRef.current().then(
      (data) => put(key, { data, loading: false, stale: false }),
      (error: unknown) =>
        put(key, {
          data: previous?.data,
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
