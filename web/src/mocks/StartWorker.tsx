"use client";

import { useEffect } from "react";
import { fileUrl, openFile, readFileText, uploadFile } from "@/lib/files";
import { getDb } from "./db";
import { worker } from "./browser";

// React runs effects twice in development; the worker must start once.
let started: Promise<unknown> | null = null;

export default function StartWorker({ onReady }: { onReady: () => void }) {
  useEffect(() => {
    let cancelled = false;
    started ??= worker.start({ onUnhandledFrame: "bypass", quiet: true });
    // No screen uploads or opens a file yet; the specs of e2e/files.spec.ts drive the two functions from here.
    Object.assign(window, { __vumloFiles: { uploadFile, openFile, fileUrl, readFileText, setMaxSize: (bytes: number) => (getDb().fileMax = bytes) } });
    started.then(() => {
      if (!cancelled) onReady();
    });
    return () => {
      cancelled = true;
    };
  }, [onReady]);
  return null;
}
