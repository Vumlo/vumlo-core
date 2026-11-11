/**
 * What a plugin report asks the web app to show: report display convention v1.
 *
 * A report is `{summary, data}`. `summary` is always shown. When `data.display` is a valid object it adds a level
 * badge, a title, a table, facts and links. Everything here is plain data: the strings reach the page as text,
 * never as HTML or markdown. A `display` that does not fit the convention is not shown at all (`null`), and the
 * panel falls back to the summary and the raw JSON.
 */

export const LEVELS = ["ok", "info", "warn", "alert"] as const;
export type Level = (typeof LEVELS)[number];

export const MAX_ROWS = 200;
export const MAX_FACTS = 20;
export const MAX_LINKS = 20;
export const MAX_TITLE = 80;
/** Not in the convention, but a renderer must not draw megabytes: longer text is cut, a wider table is invalid. */
export const MAX_COLUMNS = 30;
export const MAX_TEXT = 2000;
export const MAX_SUMMARY = 10_000;
export const MAX_RAW = 20_000;

/** `text` cut to `max` characters (code points, so no half of a pair), with an ellipsis when something was cut. */
export function clip(text: string, max: number): string {
  if (text.length <= max) return text;
  const points = Array.from(text.slice(0, 2 * max + 2));
  return points.length <= max ? text : `${points.slice(0, max).join("")}…`;
}

export type Cell = string | number;
export type Display = {
  level: Level;
  /** The plugin's own heading, or null: the panel then names the report by its plugin and key. */
  title: string | null;
  table: { columns: string[]; rows: Cell[][]; hidden: number } | null;
  facts: { label: string; value: Cell }[];
  /** Only paths inside the plugin's own routes; `href` is what an anchor takes. */
  links: { title: string; href: string }[];
};

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isCell = (v: unknown): v is Cell => typeof v === "string" || (typeof v === "number" && Number.isFinite(v));
const isText = (v: unknown): v is string => typeof v === "string";
const cut = (s: string) => clip(s, MAX_TEXT);

/**
 * True for a path under `/api/v1/plugins/<plugin id>/` and nothing else: no other plugin, no other API, no
 * address with a host, no `.` or `..` segment (also written as `%2e`), no escaped slash, no space or control character.
 */
export function isPluginPath(pluginId: string, path: unknown): path is string {
  if (typeof path !== "string" || !/^[a-z0-9][a-z0-9_-]*$/.test(pluginId)) return false;
  const prefix = `/api/v1/plugins/${pluginId}/`;
  // Not allowed anywhere: control characters, space, backslash and a `#` (a link goes to the core, not to a part of a page).
  if (!path.startsWith(prefix) || path.length === prefix.length || /[\u0000- \u007f\\#]/.test(path)) return false;
  const [route = ""] = path.split("?", 1);
  return !route.split("/").some((segment) => /^(\.|%2e){1,2}$/i.test(segment)) && !/%2f|%5c/i.test(route);
}

/** The display of `data`, or null when it is missing or does not follow the convention. Never throws. */
export function parseDisplay(pluginId: string, data: unknown): Display | null {
  if (!isObject(data) || !isObject(data.display)) return null;
  const d = data.display;

  const level = d.level === undefined ? "info" : d.level;
  if (!LEVELS.includes(level as Level)) return null;

  if (d.title !== undefined && !isText(d.title)) return null;
  const title = d.title === undefined || d.title.trim() === "" ? null : d.title.trim().slice(0, MAX_TITLE);

  let table: Display["table"] = null;
  if (d.table !== undefined) {
    if (!isObject(d.table) || !Array.isArray(d.table.columns) || !Array.isArray(d.table.rows)) return null;
    const columns = d.table.columns;
    const rows = d.table.rows;
    if (columns.length === 0 || columns.length > MAX_COLUMNS || !columns.every(isText)) return null;
    if (!rows.every((r) => Array.isArray(r) && r.length === columns.length && r.every(isCell))) return null;
    table = {
      columns: (columns as string[]).map(cut),
      rows: (rows as Cell[][]).slice(0, MAX_ROWS).map((r) => r.map((c) => (typeof c === "string" ? cut(c) : c))),
      hidden: Math.max(0, rows.length - MAX_ROWS),
    };
  }

  let facts: Display["facts"] = [];
  if (d.facts !== undefined) {
    if (!Array.isArray(d.facts) || !d.facts.every((f) => isObject(f) && isText(f.label) && isCell(f.value))) return null;
    facts = (d.facts as { label: string; value: Cell }[]).slice(0, MAX_FACTS).map((f) => ({ label: cut(f.label), value: typeof f.value === "string" ? cut(f.value) : f.value }));
  }

  let links: Display["links"] = [];
  if (d.links !== undefined) {
    if (!Array.isArray(d.links) || !d.links.every((l) => isObject(l) && isText(l.title) && isText(l.path))) return null;
    // An entry that points anywhere but at the plugin's own routes is left out; the rest of the display stays.
    links = (d.links as { title: string; path: string }[])
      .filter((l) => l.title.trim() !== "" && isPluginPath(pluginId, l.path))
      .slice(0, MAX_LINKS)
      .map((l) => ({ title: cut(l.title.trim()), href: l.path }));
  }

  return { level: level as Level, title, table, facts, links };
}

/** A number in the viewer's language; text as it is. */
export function formatCell(value: Cell, locale: string): string {
  return typeof value === "number" ? new Intl.NumberFormat(locale, { maximumFractionDigits: 6 }).format(value) : value;
}

/** The raw `data` as indented JSON for the fallback view, cut at MAX_RAW; empty when it cannot be written (too deep). */
export function rawJson(data: unknown): string {
  try {
    return clip(JSON.stringify(data, null, 2) ?? "", MAX_RAW);
  } catch {
    return "";
  }
}
