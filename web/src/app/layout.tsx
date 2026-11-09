import type { Metadata, Viewport } from "next";
import { JetBrains_Mono, Onest } from "next/font/google";
import { Suspense, type ReactNode } from "react";
import { MockProvider } from "@/mocks/MockProvider";
import { appName } from "@/lib/appName";
import { clientConfig } from "@/lib/clientConfig";
import { PageTitle } from "@/components/PageTitle";
import { LanguageProvider } from "@/lib/i18n";
import { ThemeProvider, themeInitScript } from "@/lib/theme";
import "@/styles/globals.css";

const onest = Onest({ subsets: ["latin", "cyrillic"], variable: "--font-onest", display: "swap" });
const mono = JetBrains_Mono({ subsets: ["latin", "cyrillic"], variable: "--font-mono", display: "swap" });

// The icons are always Vumlo's. There is no `title` here: Next would put it back after every navigation, and the title of the
// tab is PageTitle's. The name of the app (from client-config.json, which `vumlo-cli brand apply` generated from vumlo.toml)
// is in the page itself as `application-name`, so the HTML of the app says whose it is before any script runs.
export const metadata: Metadata = {
  applicationName: appName,
  manifest: "/brand/site.webmanifest",
  icons: {
    icon: [
      { url: "/brand/favicon.ico", sizes: "any" },
      { url: "/brand/icon-192.png", sizes: "192x192", type: "image/png" },
    ],
    apple: "/brand/apple-touch-icon.png",
  },
};
export const viewport: Viewport = { width: "device-width", initialScale: 1 };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${onest.variable} ${mono.variable}`} data-accent={clientConfig.public.institution.accent} suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeInitScript }} />
      </head>
      <body>
        <ThemeProvider>
          <MockProvider>
            <LanguageProvider>
              <Suspense fallback={null}>
                <PageTitle />
              </Suspense>
              {children}
            </LanguageProvider>
          </MockProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
