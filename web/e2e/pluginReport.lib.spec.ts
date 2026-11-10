import { expect, test } from "@playwright/test";
import { formatCell, isPluginPath, MAX_FACTS, MAX_ROWS, MAX_TITLE, parseDisplay } from "../src/lib/pluginReport";

// The display convention v1 (lib/pluginReport.ts): what is shown, what is left out, what makes it fall back.

const ID = "gradebook-export";
const show = (display: unknown) => parseDisplay(ID, { display });

test.describe("parseDisplay", () => {
  test("data with no display, or a display that is not an object, is null", () => {
    expect(parseDisplay(ID, {})).toBeNull();
    expect(parseDisplay(ID, null)).toBeNull();
    expect(parseDisplay(ID, "text")).toBeNull();
    expect(parseDisplay(ID, [])).toBeNull();
    expect(show("loud")).toBeNull();
    expect(show([])).toBeNull();
    expect(show(null)).toBeNull();
  });

  test("an empty display is valid: level info, nothing else", () => {
    expect(show({})).toEqual({ level: "info", title: null, table: null, facts: [], links: [] });
  });

  test("a full display is read as it is", () => {
    const d = show({
      level: "warn",
      title: " Risk ",
      table: { columns: ["Student", "Risk"], rows: [["Olena S.", "high"], ["Taras H.", 3]] },
      facts: [{ label: "Mean", value: "71.4" }, { label: "N", value: 24 }],
      links: [{ title: "Download XLSX", path: `/api/v1/plugins/${ID}/export/c1/xlsx` }],
    });
    expect(d).toEqual({
      level: "warn",
      title: "Risk",
      table: { columns: ["Student", "Risk"], rows: [["Olena S.", "high"], ["Taras H.", 3]], hidden: 0 },
      facts: [{ label: "Mean", value: "71.4" }, { label: "N", value: 24 }],
      links: [{ title: "Download XLSX", href: `/api/v1/plugins/${ID}/export/c1/xlsx` }],
    });
  });

  test("a level outside the four makes the display invalid", () => {
    for (const level of ["ok", "info", "warn", "alert"]) expect(show({ level })?.level).toBe(level);
    expect(show({ level: "critical" })).toBeNull();
    expect(show({ level: 1 })).toBeNull();
  });

  test("a title is cut to 80 characters and a blank one is no title", () => {
    expect(show({ title: "x".repeat(200) })?.title).toHaveLength(MAX_TITLE);
    expect(show({ title: "   " })?.title).toBeNull();
    expect(show({ title: 7 })).toBeNull();
  });

  test("a table must be rectangular, with text columns and cells of text or finite numbers", () => {
    expect(show({ table: { columns: ["A", "B"], rows: [[1]] } })).toBeNull();
    expect(show({ table: { columns: ["A"], rows: [[{ x: 1 }]] } })).toBeNull();
    expect(show({ table: { columns: ["A"], rows: [[null]] } })).toBeNull();
    expect(show({ table: { columns: ["A"], rows: [[Number.POSITIVE_INFINITY]] } })).toBeNull();
    expect(show({ table: { columns: [], rows: [] } })).toBeNull();
    expect(show({ table: { columns: [1], rows: [] } })).toBeNull();
    expect(show({ table: { columns: ["A"] } })).toBeNull();
    expect(show({ table: { columns: ["A"], rows: [] } })?.table).toEqual({ columns: ["A"], rows: [], hidden: 0 });
  });

  test("a table shows 200 rows and counts the rest", () => {
    const rows = Array.from({ length: MAX_ROWS + 37 }, (_, i) => [i]);
    const t = show({ table: { columns: ["n"], rows } })?.table;
    expect(t?.rows).toHaveLength(MAX_ROWS);
    expect(t?.hidden).toBe(37);
    expect(t?.rows.at(-1)).toEqual([MAX_ROWS - 1]);
  });

  test("facts are checked and capped at 20", () => {
    expect(show({ facts: [{ label: "a" }] })).toBeNull();
    expect(show({ facts: "mean" })).toBeNull();
    const facts = Array.from({ length: 30 }, (_, i) => ({ label: `f${i}`, value: i }));
    expect(show({ facts })?.facts).toHaveLength(MAX_FACTS);
  });

  test("a link that leaves the plugin is dropped and the rest of the display stays", () => {
    const d = show({
      level: "ok",
      links: [
        { title: "Mine", path: `/api/v1/plugins/${ID}/export` },
        { title: "Other plugin", path: "/api/v1/plugins/other/export" },
        { title: "Prefix trick", path: `/api/v1/plugins/${ID}-evil/export` },
        { title: "Host", path: "https://example.com/" },
        { title: "Scheme", path: "javascript:alert(1)" },
        { title: "", path: `/api/v1/plugins/${ID}/blank` },
      ],
    });
    expect(d?.level).toBe("ok");
    expect(d?.links).toEqual([{ title: "Mine", href: `/api/v1/plugins/${ID}/export` }]);
  });

  test("a link whose parts are not text makes the display invalid", () => {
    expect(show({ links: [{ title: "x" }] })).toBeNull();
    expect(show({ links: [{ title: 1, path: "/" }] })).toBeNull();
  });
});

test.describe("isPluginPath", () => {
  const ok = (p: string) => isPluginPath("p-1", p);
  test("accepts a route and a query of the same plugin", () => {
    expect(ok("/api/v1/plugins/p-1/export")).toBe(true);
    expect(ok("/api/v1/plugins/p-1/export/c/csv?lang=uk&x=1")).toBe(true);
  });
  test("refuses the bare prefix, other plugins and other APIs", () => {
    expect(ok("/api/v1/plugins/p-1/")).toBe(false);
    expect(ok("/api/v1/plugins/p-1")).toBe(false);
    expect(ok("/api/v1/plugins/p-10/x")).toBe(false);
    expect(ok("/api/v1/me")).toBe(false);
    expect(ok("//evil.example/api/v1/plugins/p-1/x")).toBe(false);
    expect(ok("http://x/api/v1/plugins/p-1/x")).toBe(false);
  });
  test("refuses ways out of the prefix", () => {
    expect(ok("/api/v1/plugins/p-1/../p-2/x")).toBe(false);
    expect(ok("/api/v1/plugins/p-1/%2e%2e/p-2/x")).toBe(false);
    expect(ok("/api/v1/plugins/p-1/%2E./x")).toBe(false);
    expect(ok("/api/v1/plugins/p-1/a%2Fb")).toBe(false);
    expect(ok("/api/v1/plugins/p-1/a%5Cb")).toBe(false);
    expect(ok("/api/v1/plugins/p-1/a\\b")).toBe(false);
    expect(ok("/api/v1/plugins/p-1/a b")).toBe(false);
    expect(ok("/api/v1/plugins/p-1/a\nb")).toBe(false);
    expect(ok("/api/v1/plugins/p-1/a#frag")).toBe(false);
  });
  test("a dot inside a name or in the query is fine", () => {
    expect(ok("/api/v1/plugins/p-1/anomalies.csv")).toBe(true);
    expect(ok("/api/v1/plugins/p-1/x?next=../y")).toBe(true);
  });
  test("a plugin id that is not a slug gets no links at all", () => {
    expect(isPluginPath("../x", "/api/v1/plugins/../x/y")).toBe(false);
    expect(isPluginPath("", "/api/v1/plugins//y")).toBe(false);
  });
});

test.describe("formatCell", () => {
  test("text stays as it is", () => {
    expect(formatCell("<b>1234.5</b>", "uk-UA")).toBe("<b>1234.5</b>");
    expect(formatCell("1234.5", "uk-UA")).toBe("1234.5");
  });
  test("a number follows the language", () => {
    expect(formatCell(1234.5, "en-GB")).toBe("1,234.5");
    // Ukrainian groups with a no-break space and uses a comma for the decimals.
    expect(formatCell(1234.5, "uk-UA").replace(/\s/g, " ")).toBe("1 234,5");
  });
});
