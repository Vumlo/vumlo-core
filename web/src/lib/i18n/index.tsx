"use client";

import { createContext, Fragment, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { useInstitution } from "@/lib/api/hooks/institution";
import { setTimeZone } from "@/lib/format";
import {
  language,
  setCurrentLanguage,
  storeLanguage,
  storedLanguage,
  translate,
  translatePlural,
  type Language,
  type MessageKey,
  type PluralKey,
} from "./core";

export { LANGUAGES, type Language } from "./core";

type Vars = Record<string, string | number>;
type T = {
  (key: MessageKey, vars?: Vars): string;
  /** `tn("courses.students", 3)` picks the plural form of the current language. */
  plural: (base: PluralKey, n: number, vars?: Vars) => string;
};
type Ctx = { language: Language; setLanguage: (l: Language) => void; t: T };

const make = (): T => Object.assign((key: MessageKey, vars?: Vars) => translate(key, vars), { plural: translatePlural });
const LanguageContext = createContext<Ctx>({ language: "en", setLanguage: () => {}, t: make() });

/**
 * The language is the one picked on this device, otherwise the institution's default (Admin, Institution).
 * Changing it remounts the tree, so text built by the plain functions in lib/format follows too.
 */
export function LanguageProvider({ children }: { children: ReactNode }) {
  const institution = useInstitution();
  const [picked, setPicked] = useState<Language | null>(null);
  const [read, setRead] = useState(false);

  useEffect(() => {
    // eslint-disable-next-line react-hooks/set-state-in-effect -- localStorage is only readable in the browser, after hydration
    setPicked(storedLanguage());
    setRead(true);
  }, []);

  const active: Language = picked ?? institution.data?.language ?? "en";
  setCurrentLanguage(active);
  const zone = institution.data?.time_zone;
  if (zone) setTimeZone(zone);

  useEffect(() => {
    document.documentElement.lang = active;
  }, [active]);

  const setLanguage = useCallback((l: Language) => {
    storeLanguage(l);
    setCurrentLanguage(l);
    setPicked(l);
  }, []);

  const [t] = useState(make);
  // Wait for the institution default only when the person has not picked a language, so the first paint is in the right one.
  if (!read || (picked === null && institution.isLoading)) return null;
  return (
    <LanguageContext.Provider value={{ language: active, setLanguage, t }}>
      <Fragment key={`${active}|${zone ?? ""}`}>{children}</Fragment>
    </LanguageContext.Provider>
  );
}

export function useLanguage() {
  const { language: l, setLanguage } = useContext(LanguageContext);
  return { language: l, setLanguage };
}

export function useT(): T {
  return useContext(LanguageContext).t;
}

export { language };
