"use client";

import { useEffect } from "react";
import { pdfPreview } from "@/components/grading/FilePreview";
import { fileUrl, openFile, readFileText, uploadFile } from "@/lib/files";
import { getDb } from "./db";
import { worker } from "./browser";
import { uploadDelay } from "./handlers/files";

// React runs effects twice in development; the worker must start once.
let started: Promise<unknown> | null = null;

export default function StartWorker({ onReady }: { onReady: () => void }) {
  useEffect(() => {
    let cancelled = false;
    started ??= worker.start({ onUnhandledFrame: "bypass", quiet: true });
    // The specs of e2e/files.spec.ts drive the two functions from here; the editor specs also slow uploads down and sweep a file.
    Object.assign(window, { __vumloFiles: { uploadFile, openFile, fileUrl, readFileText, setMaxSize: (bytes: number) => (getDb().fileMax = bytes), setUploadDelay: (ms: number) => (uploadDelay.ms = ms), sweep: (id: string) => getDb().files.delete(id), setPdfIframe: (on: boolean) => (pdfPreview.iframe = on) } });
    started.then(() => {
      if (!cancelled) onReady();
    });
    return () => {
      cancelled = true;
    };
  }, [onReady]);
  return null;
}
