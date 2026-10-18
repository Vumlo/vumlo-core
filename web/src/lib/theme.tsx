"use client";

import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";

export type Theme = "light" | "dark";
export type ThemePreference = Theme | "system";

const KEY = "vumlo.theme";

/** Runs before first paint, so the page never flashes the wrong theme. */
export const themeInitScript = `(function(){try{var t=localStorage.getItem("${KEY}");if(t!=="light"&&t!=="dark"){t=matchMedia("(prefers-color-scheme: dark)").matches?"dark":"light"}document.documentElement.dataset.theme=t}catch(e){document.documentElement.dataset.theme="light"}})()`;

type Ctx = { theme: Theme; preference: ThemePreference; toggle: () => void; setPreference: (p: ThemePreference) => void };

const ThemeContext = createContext<Ctx>({ theme: "light", preference: "system", toggle: () => {}, setPreference: () => {} });

function read(): ThemePreference {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

function resolve(p: ThemePreference): Theme {
  if (p !== "system") return p;
  return matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [preference, setPref] = useState<ThemePreference>("system");
  const [theme, setTheme] = useState<Theme>("light");

  useEffect(() => {
    const p = read();
    // eslint-disable-next-line react-hooks/set-state-in-effect -- syncs with the value the init script already applied
    setPref(p);
    setTheme(resolve(p));
    const media = matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => {
      if (read() === "system") {
        const t = resolve("system");
        document.documentElement.dataset.theme = t;
        setTheme(t);
      }
    };
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  const setPreference = useCallback((p: ThemePreference) => {
    try {
      if (p === "system") localStorage.removeItem(KEY);
      else localStorage.setItem(KEY, p);
    } catch {
      // Storage can be blocked; the theme still applies for this visit.
    }
    const t = resolve(p);
    document.documentElement.dataset.theme = t;
    setPref(p);
    setTheme(t);
  }, []);

  const toggle = useCallback(() => {
    const next: Theme = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    setPreference(next);
  }, [setPreference]);

  return <ThemeContext.Provider value={{ theme, preference, toggle, setPreference }}>{children}</ThemeContext.Provider>;
}

export const useTheme = () => useContext(ThemeContext);
