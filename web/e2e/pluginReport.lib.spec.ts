import { expect, test } from "@playwright/test";
import { clip, formatCell, isPluginPath, MAX_COLUMNS, MAX_FACTS, MAX_RAW, MAX_ROWS, MAX_TEXT, MAX_TITLE, parseDisplay, rawJson } from "../src/lib/pluginReport";

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

// Hostile input: the parser must never throw, and must bound what it hands to the renderer.
test.describe("hostile reports", () => {
  test("script and markup in any string stay plain strings", () => {
    const evil = `<script>alert(1)</script><img src=x onerror=alert(2)>`;
    const d = show({
      title: evil,
      table: { columns: [evil], rows: [[evil]] },
      facts: [{ label: evil, value: evil }],
      links: [{ title: evil, path: `/api/v1/plugins/${ID}/x` }],
    });
    expect(d?.title).toBe(evil);
    expect(d?.table?.rows[0]).toEqual([evil]);
    expect(d?.links[0]?.title).toBe(evil);
  });

  test("javascript:, data:, protocol-relative and other-plugin links are all dropped", () => {
    const paths = [
      "javascript:alert(1)", " javascript:alert(1)", "JaVaScRiPt:alert(1)", "data:text/html,<script>1</script>",
      "//evil.example/x", "/\\evil.example", "/api/v1/plugins/other/x", "/api/v1/plugins/../other/x",
      `/api/v1/plugins/${ID}/\tjavascript:alert(1)`, `/api/v1/plugins/${ID}/%2e%2e/other/x`,
    ];
    const d = show({ links: paths.map((path) => ({ title: "go", path })) });
    expect(d?.links).toEqual([]);
  });

  test("10 000 rows: 200 are kept, the rest is counted", () => {
    const rows = Array.from({ length: 10_000 }, (_, i) => [`s${i}`, i]);
    const t = show({ table: { columns: ["a", "b"], rows } })?.table;
    expect(t?.rows).toHaveLength(MAX_ROWS);
    expect(t?.hidden).toBe(10_000 - MAX_ROWS);
  });

  test("a table of 10 000 columns is invalid, not rendered", () => {
    const columns = Array.from({ length: 10_000 }, (_, i) => `c${i}`);
    expect(show({ table: { columns, rows: [] } })).toBeNull();
    expect(show({ table: { columns: columns.slice(0, MAX_COLUMNS), rows: [] } })).not.toBeNull();
  });

  test("huge strings are cut to MAX_TEXT, by code points", () => {
    const huge = "x".repeat(5_000_000);
    const d = show({
      table: { columns: [huge], rows: [[huge]] },
      facts: [{ label: huge, value: huge }],
      links: [{ title: huge, path: `/api/v1/plugins/${ID}/x` }],
    });
    for (const s of [d!.table!.columns[0]!, d!.table!.rows[0]![0] as string, d!.facts[0]!.label, d!.facts[0]!.value as string, d!.links[0]!.title]) {
      expect(s).toHaveLength(MAX_TEXT + 1);
      expect(s.endsWith("…")).toBe(true);
    }
    // An emoji on the cut is not split into half a surrogate pair.
    const cut = clip("😀".repeat(10), 3);
    expect(cut).toBe("😀😀😀…");
    expect(clip("short", 10)).toBe("short");
  });

  test("garbage of every type in display never throws and gives null or a valid display", () => {
    const garbage: unknown[] = [
      null, undefined, true, 0, -1, "", "x", [], [[]], {}, { a: { b: { c: [] } } }, [null], Symbol.iterator.toString(),
      { __proto__: { level: "alert" } }, JSON.parse('{"__proto__": {"level": "alert"}}'), JSON.parse('{"constructor": 1}'),
    ];
    for (const g of garbage) {
      for (const field of ["level", "title", "table", "facts", "links"]) {
        expect(() => show({ [field]: g })).not.toThrow();
      }
      expect(() => parseDisplay(ID, { display: g })).not.toThrow();
      expect(() => parseDisplay(ID, g)).not.toThrow();
    }
    // Inherited properties do not count as the report's own.
    expect(show(JSON.parse('{"__proto__": {"level": "alert"}}'))?.level).toBe("info");
  });

  test("a display nested 100 000 levels deep does not blow the stack, and the raw view is empty or cut", () => {
    let deep: unknown = "end";
    for (let i = 0; i < 100_000; i++) deep = [deep];
    expect(() => show({ table: deep })).not.toThrow();
    expect(show({ table: deep })).toBeNull();
    expect(() => rawJson({ display: { table: deep } })).not.toThrow();
    expect(rawJson({ display: { table: deep } }).length).toBeLessThanOrEqual(MAX_RAW + 1);
  });

  test("a circular value does not throw in rawJson", () => {
    const a: Record<string, unknown> = {};
    a.self = a;
    expect(rawJson(a)).toBe("");
  });

  test("the raw view is cut", () => {
    const out = rawJson({ big: "y".repeat(1_000_000) });
    expect(out.length).toBe(MAX_RAW + 1);
  });

  test("negative, zero, huge, tiny and non-finite numbers", () => {
    expect(formatCell(-1234.5, "en-GB")).toBe("-1,234.5");
    expect(formatCell(0, "en-GB")).toBe("0");
    expect(() => formatCell(-0, "en-GB")).not.toThrow();
    expect(() => formatCell(1e308, "en-GB")).not.toThrow();
    expect(() => formatCell(5e-324, "uk-UA")).not.toThrow();
    expect(formatCell(0.1 + 0.2, "en-GB")).toBe("0.3");
    // JSON cannot carry NaN or Infinity (they become null); a value that still gets here makes the table invalid.
    for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY, null]) {
      expect(show({ table: { columns: ["a"], rows: [[bad]] } })).toBeNull();
      expect(show({ facts: [{ label: "a", value: bad }] })).toBeNull();
    }
    expect(show({ table: { columns: ["a"], rows: [[-5], [-0.5], [-1e21]] } })?.table?.rows).toEqual([[-5], [-0.5], [-1e21]]);
  });
});
