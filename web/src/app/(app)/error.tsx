"use client";

import { ErrorScreen } from "@/components/StateScreen";

export default function AppError({ reset }: { error: Error & { digest?: string }; reset: () => void }) {
  return <ErrorScreen reset={reset} />;
}
