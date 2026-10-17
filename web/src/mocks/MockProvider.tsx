"use client";

import dynamic from "next/dynamic";
import { useState, type ReactNode } from "react";

const enabled = process.env.NEXT_PUBLIC_API_MOCKING === "enabled";

// The worker module is browser-only, so it is kept out of the server render.
const StartWorker = dynamic(() => import("./StartWorker"), { ssr: false });

/** Starts the MSW worker before the first request when the app runs under `pnpm dev:mock`. */
export function MockProvider({ children }: { children: ReactNode }) {
  const [ready, setReady] = useState(!enabled);
  return (
    <>
      {enabled && !ready && <StartWorker onReady={() => setReady(true)} />}
      {ready ? children : null}
    </>
  );
}
