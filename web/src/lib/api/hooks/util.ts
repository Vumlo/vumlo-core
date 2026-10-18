import { api, type Page } from "../client";

/** Follows `next_cursor` until the list ends. The mock and the server both page with `limit`. */
export async function fetchAll<T>(path: string): Promise<T[]> {
  const out: T[] = [];
  let cursor: string | null = null;
  const sep = path.includes("?") ? "&" : "?";
  do {
    const page: Page<T> = await api.get<Page<T>>(`${path}${sep}limit=100${cursor ? `&cursor=${encodeURIComponent(cursor)}` : ""}`);
    out.push(...page.items);
    cursor = page.next_cursor;
  } while (cursor);
  return out;
}
