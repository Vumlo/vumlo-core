"use client";

import { useEffect, useState } from "react";

/** Renders again every `ms`, so a screen that depends on the time (a deadline) does not go stale while open. */
export function useTick(ms = 30_000) {
  const [, set] = useState(0);
  useEffect(() => {
    const timer = setInterval(() => set((n) => n + 1), ms);
    return () => clearInterval(timer);
  }, [ms]);
}
