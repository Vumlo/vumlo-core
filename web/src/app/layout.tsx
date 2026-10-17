import type { Metadata, Viewport } from "next";
import { JetBrains_Mono, Onest } from "next/font/google";
import type { ReactNode } from "react";
import { MockProvider } from "@/mocks/MockProvider";
import { ThemeProvider, themeInitScript } from "@/lib/theme";
import "@/styles/globals.css";

const onest = Onest({ subsets: ["latin", "cyrillic"], variable: "--font-onest", display: "swap" });
const mono = JetBrains_Mono({ subsets: ["latin", "cyrillic"], variable: "--font-mono", display: "swap" });

export const metadata: Metadata = { title: { default: "Learnly", template: "%s · Learnly" } };
export const viewport: Viewport = { width: "device-width", initialScale: 1 };

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${onest.variable} ${mono.variable}`} suppressHydrationWarning>
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeInitScript }} />
      </head>
      <body>
        <ThemeProvider>
          <MockProvider>{children}</MockProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
