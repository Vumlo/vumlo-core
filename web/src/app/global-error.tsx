"use client";

import { translate } from "@/lib/i18n/core";

/**
 * The last resort when the root layout itself fails, so no providers and no stylesheet are available.
 * It uses only browser defaults, which follow the system light or dark setting.
 */
export default function GlobalError({ reset }: { error: Error & { digest?: string }; reset: () => void }) {
  return (
    <html lang="en" style={{ colorScheme: "light dark" }}>
      <body style={{ fontFamily: "system-ui, sans-serif", margin: 0 }}>
        <main role="alert" style={{ maxWidth: 520, margin: "0 auto", padding: "96px 16px", textAlign: "center" }}>
          <h1 style={{ fontSize: 26, margin: "0 0 8px" }}>{translate("error.title")}</h1>
          <p style={{ margin: "0 0 24px" }}>{translate("error.message")}</p>
          <button type="button" onClick={reset} style={{ minHeight: 44, padding: "0 20px", fontSize: 16 }}>{translate("error.retry")}</button>
        </main>
      </body>
    </html>
  );
}
