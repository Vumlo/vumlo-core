import { http, HttpResponse } from "msw";
import { MOCK_NOW } from "@/lib/clock";
import type { PluginReport } from "@/lib/api/types";
import { getDb } from "../db";
import { courseById, itemById, notFound, requireStaff } from "../logic";

const A = "/api/v1";
const at = MOCK_NOW.toISOString();

/** A report as the server returns it; `data` is whatever the plugin wrote. */
const report = (plugin_id: string, key: string, summary: string, data: Record<string, unknown>): PluginReport => ({
  plugin_id, key, summary, updated_at: at, data: data as PluginReport["data"],
});

/** One submitted attempt: a full display, and a report from a plugin that does not use the convention. */
const forSubmission = (): PluginReport[] => [
  report("text-metrics", "text-metrics", "412 words, about 2 minutes to read. Below the minimum of 500 words.", {
    display: {
      level: "warn",
      title: "Length of the text",
      facts: [{ label: "Words", value: 412 }, { label: "Characters", value: 2845 }, { label: "Reading time", value: "2 min" }, { label: "Minimum", value: 500 }],
    },
  }),
  report("legacy-check", "legacy", "Nothing suspicious <b>found</b> & **checked**.", { checked: 3, hits: [] }),
];

/** One assignment: a table with a link that is the plugin's own, and two that are not (they are left out). */
const forAssignment = (): PluginReport[] => [
  report("grade-anomaly-report", "anomalies", "2 grades look unusual out of 24.", {
    display: {
      level: "alert",
      title: "Unusual grades",
      table: {
        columns: ["Student", "Points", "Why"],
        rows: [["Olena S.", 3.5, "Far below the group"], ["Taras H.", 100, "Identical to 5 others"]],
      },
      facts: [{ label: "Median", value: 71.5 }, { label: "Mean", value: 70.4 }],
      links: [
        { title: "Download the list", path: "/api/v1/plugins/grade-anomaly-report/export/anomalies.csv" },
        { title: "Another plugin", path: "/api/v1/plugins/other-plugin/export" },
        { title: "Elsewhere", path: "https://example.com/steal" },
      ],
    },
  }),
  report("quiz-item-analysis", "broken", "Analysis is not ready.", { display: { table: { columns: ["A", "B"], rows: [[1]] } } }),
];

const flagged = () => {
  try {
    return localStorage.getItem("vumlo.mock.hostileReports") === "1";
  } catch {
    return false;
  }
};

const XSS =`<script>window.__xss = 1</script><img src=x onerror="window.__xss = 2">`;

/** Reports a hostile or broken plugin could write (the panel must show them as text and not fall over). */
const hostile = (courseId: string): PluginReport[] => {
  let deep: unknown = "end";
  for (let i = 0; i < 200; i++) deep = { a: [deep] };
  return [
    report("hostile-html", "html", XSS, {
      display: {
        level: "alert",
        title: XSS,
        table: { columns: [XSS], rows: [[XSS], [-12.5], [-0]] },
        facts: [{ label: XSS, value: XSS }, { label: "Negative", value: -1234.5 }],
        links: [
          { title: "Script", path: "javascript:window.__xss = 3" },
          { title: "Other plugin", path: `/api/v1/plugins/gradebook-export/export/${courseId}/csv` },
          { title: "Host", path: "//evil.example/x" },
          { title: "Mine", path: "/api/v1/plugins/hostile-html/export" },
        ],
      },
    }),
    report("hostile-big", "big", "s".repeat(300_000), {
      display: {
        title: "Big",
        table: { columns: ["n", "text"], rows: Array.from({ length: 10_000 }, (_, i) => [i, i === 0 ? "w".repeat(500_000) : "ok"]) },
      },
    }),
    report("hostile-deep", "deep", "Nested garbage.", { display: { level: deep, table: deep, facts: deep }, deep }),
  ];
};

/** One course: links to an export, and a table longer than the 200 rows the panel shows. */
const forCourse = (courseId: string): PluginReport[] => [
  // Only for the tests of the panel, which set this key (e2e/pluginReports.hostile.spec.ts).
  ...(flagged() ? hostile(courseId) : []),
  report("gradebook-export", "gradebook", "The gradebook is ready for download.", {
    display: {
      level: "ok",
      title: "Gradebook export",
      links: [
        { title: "Download CSV", path: `/api/v1/plugins/gradebook-export/export/${courseId}/csv` },
        { title: "Download XLSX", path: `/api/v1/plugins/gradebook-export/export/${courseId}/xlsx` },
      ],
    },
  }),
  report("at-risk-students", "at-risk", "230 students are listed by risk.", {
    display: {
      level: "info",
      title: "At-risk students",
      table: {
        columns: ["Student", "Average, %", "Risk"],
        rows: Array.from({ length: 230 }, (_, i) => [`Student ${i + 1}`, 1000 + i * 12.5, i % 3 === 0 ? "high" : "low"]),
      },
    },
  }),
];

export const pluginReportHandlers = [
  http.get(`${A}/submissions/:id/plugin-reports`, ({ params }) => {
    const id = String(params.id);
    const sub = getDb().submissions.find((s) => s.attempts.some((a) => a.id === id));
    if (!sub) throw notFound();
    requireStaff(itemById(sub.assignment_id).course_id);
    return HttpResponse.json({ items: forSubmission() });
  }),
  http.get(`${A}/assignments/:id/plugin-reports`, ({ params }) => {
    const item = itemById(String(params.id));
    if (!item.assignment) throw notFound();
    requireStaff(item.course_id);
    return HttpResponse.json({ items: forAssignment() });
  }),
  http.get(`${A}/courses/:id/plugin-reports`, ({ params }) => {
    const course = courseById(String(params.id));
    requireStaff(course.id);
    return HttpResponse.json({ items: forCourse(course.id) });
  }),
  // What the link of the course report leads to: a file the browser downloads with the session.
  http.get(`${A}/plugins/gradebook-export/export/:course/csv`, () =>
    new HttpResponse("Student,Total\nOlena S.,71.5\n", { headers: { "Content-Type": "text/csv", "Content-Disposition": 'attachment; filename="gradebook.csv"' } }),
  ),
];
