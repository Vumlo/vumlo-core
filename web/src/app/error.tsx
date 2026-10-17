"use client";

import { ErrorScreen } from "@/components/StateScreen";

export default function RootError({ reset }: { error: Error & { digest?: string }; reset: () => void }) {
  return (
    <main>
      <ErrorScreen reset={reset} />
    </main>
  );
}
