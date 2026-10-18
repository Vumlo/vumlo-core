"use client";

import { useEffect } from "react";
import { worker } from "./browser";

// React runs effects twice in development; the worker must start once.
let started: Promise<unknown> | null = null;

export default function StartWorker({ onReady }: { onReady: () => void }) {
  useEffect(() => {
    let cancelled = false;
    started ??= worker.start({ onUnhandledFrame: "bypass", quiet: true });
    started.then(() => {
      if (!cancelled) onReady();
    });
    return () => {
      cancelled = true;
    };
  }, [onReady]);
  return null;
}
