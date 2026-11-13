import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { clientConfig } from "@/lib/clientConfig";
import { LanguageProvider } from "@/lib/i18n";
import { ThemeProvider } from "@/lib/theme";
import "@/styles/globals.css";
import { App } from "./App";

document.documentElement.dataset.accent = clientConfig.public.institution.accent;

async function start() {
  // The mock API is for `npm run dev:mock` (`--mode mock`); a build drops this branch and with it msw.
  if (import.meta.env.MODE === "mock") await (await import("./mocks/start")).start();
  createRoot(document.getElementById("root")!).render(
    <StrictMode>
      <ThemeProvider>
        <LanguageProvider>
          <App />
        </LanguageProvider>
      </ThemeProvider>
    </StrictMode>,
  );
}

void start();
