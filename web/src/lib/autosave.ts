"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { now } from "./clock";
import { useUnloadGuard } from "./useUnloadGuard";

const RETRY_MS = 5000;

/**
 * Calls `save` a moment after `value` stops changing. The first value is not saved. `flush` saves at once and
 * resolves to false when the save failed, so a caller (Publish) does not go on with an older copy.
 * A failed save is tried again, a change still waiting is saved when the editor closes, and closing the tab asks first.
 * Editors use it so a change is stored without a Save button (Editor: "Saved 14:02").
 */
export function useAutosave<T>(value: T, save: (v: T) => Promise<unknown>, delay = 1000) {
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const initial = useRef(value);
  const latest = useRef(value);
  const dirty = useRef(false);
  const alive = useRef(true);
  const inflight = useRef<Promise<boolean> | null>(null);
  const retry = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saveRef = useRef(save);
  useEffect(() => { saveRef.current = save; });
  useUnloadGuard(() => dirty.current || inflight.current !== null);

  const run = useCallback((): Promise<boolean> => {
    if (retry.current) clearTimeout(retry.current);
    retry.current = null;
    // Saves go one after the other, so a flush also waits for the one already on its way.
    const next = (inflight.current ?? Promise.resolve(true)).then(async () => {
      if (!dirty.current) return true;
      dirty.current = false;
      setPending(true);
      try {
        const ok = await saveRef.current(latest.current);
        if (ok === undefined) {
          dirty.current = true;
          setFailed(true);
          if (alive.current) retry.current = setTimeout(() => void run(), RETRY_MS);
          return false;
        }
        setFailed(false);
        setSavedAt(now().toISOString());
        return true;
      } finally {
        setPending(false);
      }
    });
    inflight.current = next;
    void next.finally(() => {
      if (inflight.current === next) inflight.current = null;
    });
    return next;
  }, []);

  useEffect(() => {
    latest.current = value;
    // Strict mode runs effects twice in development; an unchanged value is not a change.
    if (Object.is(value, initial.current)) return;
    dirty.current = true;
    const t = setTimeout(() => void run(), delay);
    return () => clearTimeout(t);
  }, [value, delay, run]);

  // Leaving the editor within the delay still saves the last change.
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      if (retry.current) clearTimeout(retry.current);
      void run();
    };
  }, [run]);

  return { savedAt, pending, failed, flush: run };
}
