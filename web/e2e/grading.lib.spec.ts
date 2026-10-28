import { expect, test } from "@playwright/test";
import type { GradebookCell, SubmissionRow } from "../src/lib/api/types";
import { autoResult, cellState, inWorkspaceFilter, matchesStatus, neighbour, type CellState } from "../src/lib/grading";

const cell = (c: Record<string, unknown>) => c as unknown as GradebookCell;
const sub = (s: Record<string, unknown>) => s as unknown as SubmissionRow;
const c = (state: CellState, late = false) => ({ state, late });

test.describe("cellState", () => {
  test("an unpublished draft score is a draft", () => {
    expect(cellState(cell({ has_unpublished_changes: true, draft_score: "22.50" }), undefined)).toBe("draft");
  });
  test("a published score without a draft is published", () => {
    expect(cellState(cell({ published_score: "18.50", has_unpublished_changes: false, draft_score: null }), undefined)).toBe("pub");
  });
  test("a submission without a cell waits for grading", () => {
    expect(cellState(undefined, sub({ status: "submitted" }))).toBe("grade");
  });
  test("a submission whose cell has a draft score is not waiting", () => {
    expect(cellState(cell({ draft_score: "5.00", has_unpublished_changes: false }), sub({ status: "submitted" }))).not.toBe("grade");
  });
  test("nothing at all is none", () => {
    expect(cellState(undefined, undefined)).toBe("none");
  });
});

test.describe("matchesStatus", () => {
  test("all without late matches anything", () => {
    expect(matchesStatus([], { status: "all", late: false })).toBe(true);
    expect(matchesStatus([c("none")], { status: "all", late: false })).toBe(true);
  });
  test("a status needs a cell in that state", () => {
    expect(matchesStatus([c("none"), c("grade")], { status: "grade", late: false })).toBe(true);
    expect(matchesStatus([c("none"), c("pub")], { status: "grade", late: false })).toBe(false);
  });
  test("late needs a late cell", () => {
    expect(matchesStatus([c("pub", true)], { status: "all", late: true })).toBe(true);
    expect(matchesStatus([c("pub")], { status: "all", late: true })).toBe(false);
  });
  test("status and late must hold for one and the same cell", () => {
    expect(matchesStatus([c("draft", true)], { status: "draft", late: true })).toBe(true);
    expect(matchesStatus([c("draft"), c("pub", true)], { status: "draft", late: true })).toBe(false);
  });
});

test("inWorkspaceFilter", () => {
  const all: CellState[] = ["pub", "draft", "grade", "none"];
  expect(all.filter((s) => inWorkspaceFilter(s, "grade"))).toEqual(["grade"]);
  expect(all.filter((s) => inWorkspaceFilter(s, "draft"))).toEqual(["draft"]);
  expect(all.filter((s) => inWorkspaceFilter(s, "all"))).toEqual(all);
});

test.describe("neighbour", () => {
  const order = ["a", "b", "c", "d"];
  const visible = new Set(["b", "d"]);
  test("walks to the next visible id", () => {
    expect(neighbour(order, visible, "b", 1)).toBe("d");
    expect(neighbour(order, visible, "c", -1)).toBe("b");
  });
  test("stops at the ends", () => {
    expect(neighbour(order, visible, "d", 1)).toBeNull();
    expect(neighbour(order, visible, "b", -1)).toBeNull();
  });
  test("starts from a current outside the filter", () => {
    expect(neighbour(order, visible, "a", 1)).toBe("b");
  });
  test("an unknown current is null", () => {
    expect(neighbour(order, visible, "zz", 1)).toBeNull();
  });
});

test.describe("autoResult", () => {
  test("a full score makes every pick right", () => {
    expect(autoResult("multi", ["a", "b"], "2.00", "2.00")).toEqual({ correct: ["a", "b"] });
  });
  test("a zero on a single-choice question marks its pick wrong", () => {
    expect(autoResult("single", ["a"], "0.00", "1.00")).toEqual({ correct: [] });
  });
  test("a partial score on a multiple-choice question is no verdict", () => {
    expect(autoResult("multi", ["a", "b"], "1.00", "2.00")).toBeNull();
  });
  test("a zero on a multiple-choice question is no verdict", () => {
    expect(autoResult("multi", ["a"], "0.00", "2.00")).toBeNull();
  });
  test("0 and 0.00 are the same amount", () => {
    expect(autoResult("single", ["a"], "0", "0.00")).toEqual({ correct: ["a"] });
    expect(autoResult("single", ["a"], "2", "2.00")).toEqual({ correct: ["a"] });
    expect(autoResult("single", ["a"], "0", "1.00")).toEqual({ correct: [] });
  });
});
