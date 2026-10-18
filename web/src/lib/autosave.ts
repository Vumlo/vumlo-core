"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { now } from "./clock";

/**
 * Calls `save` a moment after `value` stops changing. The first value is not saved. `flush` saves at once.
 * Editors use it so a change is stored without a Save button (Editor: "Saved 14:02").
 */
export function useAutosave<T>(value: T, save: (v: T) => Promise<unknown>, delay = 1000) {
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const initial = useRef(value);
  const latest = useRef(value);
  const dirty = useRef(false);
  const saveRef = useRef(save);
  useEffect(() => { saveRef.current = save; });

  const run = useCallback(async () => {
    if (!dirty.current) return;
    dirty.current = false;
    setPending(true);
    try {
      const ok = await saveRef.current(latest.current);
      setFailed(ok === undefined);
      if (ok !== undefined) setSavedAt(now().toISOString());
    } finally {
      setPending(false);
    }
  }, []);

  useEffect(() => {
    latest.current = value;
    // Strict mode runs effects twice in development; an unchanged value is not a change.
    if (Object.is(value, initial.current)) return;
    dirty.current = true;
    const t = setTimeout(() => void run(), delay);
    return () => clearTimeout(t);
  }, [value, delay, run]);

  return { savedAt, pending, failed, flush: run };
}
