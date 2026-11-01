import type { Metadata, Viewport } from "next";
import { JetBrains_Mono, Onest } from "next/font/google";
import { Suspense, type ReactNode } from "react";
import { MockProvider } from "@/mocks/MockProvider";
import { appName } from "@/lib/appName";
import { PageTitle } from "@/components/PageTitle";
import { LanguageProvider } from "@/lib/i18n";
import { ThemeProvider, themeInitScript } from "@/lib/theme";
import "@/styles/globals.css";

const onest = Onest({ subsets: ["latin", "cyrillic"], variable: "--font-onest", display: "swap" });
const mono = JetBrains_Mono({ subsets: ["latin", "cyrillic"], variable: "--font-mono", display: "swap" });

// The name is from client-config.json, which `vumlo-cli brand apply` generated from vumlo.toml; the icons are always Vumlo's.
export const metadata: Metadata = {
  title: { default: appName, template: `%s · ${appName}` },
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
    <html lang="en" className={`${onest.variable} ${mono.variable}`} suppressHydrationWarning>
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
