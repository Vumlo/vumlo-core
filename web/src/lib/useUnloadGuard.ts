"use client";

import { useEffect, useRef } from "react";

/** Asks the browser to confirm closing or reloading the tab while `active()` is true (unsent work). */
export function useUnloadGuard(active: () => boolean) {
  const latest = useRef(active);
  useEffect(() => {
    latest.current = active;
  });
  useEffect(() => {
    const guard = (e: BeforeUnloadEvent) => {
      if (latest.current()) e.preventDefault();
    };
    window.addEventListener("beforeunload", guard);
    return () => window.removeEventListener("beforeunload", guard);
  }, []);
}
