import type { Metadata, Viewport } from "next";
import { JetBrains_Mono, Onest } from "next/font/google";
import type { ReactNode } from "react";
import { MockProvider } from "@/mocks/MockProvider";
import { clientConfig } from "@/lib/clientConfig";
import { LanguageProvider } from "@/lib/i18n";
import { ThemeProvider, themeInitScript } from "@/lib/theme";
import "@/styles/globals.css";

const onest = Onest({ subsets: ["latin", "cyrillic"], variable: "--font-onest", display: "swap" });
const mono = JetBrains_Mono({ subsets: ["latin", "cyrillic"], variable: "--font-mono", display: "swap" });

// From client-config.json, which `vumlo-cli brand apply` generated from vumlo.toml. The icons exist
// only when the institution has a logo.
const institution = clientConfig.public.institution;
export const metadata: Metadata = {
  title: { default: institution.name, template: `%s · ${institution.short_name}` },
  manifest: "/brand/site.webmanifest",
  ...(clientConfig.logo
    ? {
        icons: {
          icon: [
            { url: "/brand/favicon.ico", sizes: "any" },
            { url: "/brand/icon-192.png", sizes: "192x192", type: "image/png" },
          ],
          apple: "/brand/apple-touch-icon.png",
        },
      }
    : {}),
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
            <LanguageProvider>{children}</LanguageProvider>
          </MockProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
