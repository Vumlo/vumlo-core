import { en } from "./en";
import { uk } from "./uk";

export type Language = "en" | "uk";
export type MessageKey = keyof typeof en;
/** A plural message is stored as `base.one`, `base.few`, `base.many` and `base.other`. */
export type PluralKey = { [K in MessageKey]: K extends `${infer B}.other` ? B : never }[MessageKey];

export const LANGUAGES: { id: Language; name: string }[] = [
  { id: "en", name: "English" },
  { id: "uk", name: "Українська" },
];

const catalogues: Record<Language, Record<string, string>> = { en, uk };
const LANG_KEY = "vumlo.language";

let current: Language = "en";

export function language(): Language {
  return current;
}

export function setCurrentLanguage(l: Language) {
  current = l;
}

/** The language the person picked on this device, if any. */
export function storedLanguage(): Language | null {
  try {
    const v = localStorage.getItem(LANG_KEY);
    return v === "en" || v === "uk" ? v : null;
  } catch {
    return null;
  }
}

export function storeLanguage(l: Language) {
  try {
    localStorage.setItem(LANG_KEY, l);
  } catch {
    // Storage can be blocked; the choice then lasts until the page closes.
  }
}

function fill(text: string, vars?: Record<string, string | number>) {
  return vars ? text.replace(/\{(\w+)\}/g, (_, k: string) => String(vars[k] ?? `{${k}}`)) : text;
}

/** The message for `key` in the current language, with `{name}` placeholders filled from `vars`. */
export function translate(key: MessageKey, vars?: Record<string, string | number>) {
  return fill(catalogues[current][key] ?? catalogues.en[key] ?? key, vars);
}

/** The plural form of `base` for `n` in the current language; `{n}` is filled in. */
export function translatePlural(base: PluralKey, n: number, vars?: Record<string, string | number>) {
  const rule = new Intl.PluralRules(current).select(n);
  const cat = catalogues[current];
  const text = cat[`${base}.${rule}`] ?? cat[`${base}.other`] ?? base;
  return fill(text, { n, ...vars });
}

export const locale = () => (current === "uk" ? "uk-UA" : "en-GB");
