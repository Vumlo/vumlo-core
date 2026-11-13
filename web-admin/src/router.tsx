import { useSyncExternalStore, type ReactNode } from "react";

// A router of the size the console needs: the address bar is the state, a page is a path, and `:name` in a
// path is a parameter. No library: the console has a dozen pages and no nesting beyond its frame.

const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());
if (typeof window !== "undefined") window.addEventListener("popstate", emit);

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** `/path?query`, as the address bar has it. */
const here = () => window.location.pathname + window.location.search;

export function useLocation() {
  const href = useSyncExternalStore(subscribe, here, () => "/");
  const at = href.indexOf("?");
  return { path: at < 0 ? href : href.slice(0, at), search: at < 0 ? "" : href.slice(at) };
}

export function navigate(to: string, replace = false) {
  if (to === here()) return;
  if (replace) window.history.replaceState(null, "", to);
  else window.history.pushState(null, "", to);
  emit();
}

export type Route = { path: string; render: (params: Record<string, string>) => ReactNode };

/** `/plugins/:id` against `/plugins/ocr`: `{ id: "ocr" }`, or `null` when the path is not that. */
export function match(pattern: string, path: string): Record<string, string> | null {
  const want = pattern.split("/");
  const have = path.split("/");
  if (want.length !== have.length) return null;
  const params: Record<string, string> = {};
  for (const [i, part] of want.entries()) {
    const there = have[i] ?? "";
    if (part.startsWith(":")) params[part.slice(1)] = decodeURIComponent(there);
    else if (part !== there) return null;
  }
  return params;
}
