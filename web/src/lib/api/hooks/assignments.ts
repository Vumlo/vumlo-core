"use client";

import { api, type Queued } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import type { AnswerMap, FormConfig, FormResponses, QuizConfig, QuizResult } from "../gaps";
import { formResponsesFromWire, quizResultFromWire } from "../sheetWire";
import type { Attempt, Document, GradeHistoryEntry, GradebookPage, MySubmission, SubmissionRow } from "../types";
import { fetchAll } from "./util";

export function useMySubmission(assignmentId: string | null) {
  return useQuery<MySubmission>(assignmentId ? `/assignments/${assignmentId}/submission` : null, () => api.get<MySubmission>(`/assignments/${assignmentId}/submission`));
}

/** Quizzes and forms store their answers in the draft's `content` as `{answers}` (API_GAPS.md, "Quiz and form types"). */
export type DraftContent = Document | { answers: AnswerMap };

/** `queue`: offline on the desktop the write waits for a connection. A quiz passes false: it needs one (ADR-0060). */
export function useSaveDraft(assignmentId: string, queue = true) {
  return useMutation(async (content: DraftContent) => {
    // Offline on the desktop the draft waits in the queue and comes back as it will look once saved (ADR-0060).
    const d = await api.put<{ content: DraftContent; updated_at: string }>(`/assignments/${assignmentId}/submission/draft`, { content }, { queue });
    invalidate(`/assignments/${assignmentId}/submission`);
    invalidate("/me/work");
    return d;
  });
}

export function useSubmitDraft(assignmentId: string, queue = true) {
  return useMutation(async () => {
    const a = await api.post<Attempt | Queued>(`/assignments/${assignmentId}/submission/submit`, undefined, { queue });
    invalidate(`/assignments/${assignmentId}`);
    invalidate("/me/");
    invalidate("/courses/");
    return a;
  });
}

/** The result of a quiz once its grade is published (ADR-0050); `config` is the quiz as the screens read it. */
export function useQuizResult(assignmentId: string | null, config: QuizConfig) {
  return useQuery<QuizResult>(assignmentId ? `/assignments/${assignmentId}/result` : null, async () => quizResultFromWire(await api.get(`/assignments/${assignmentId}/result`), config));
}

export function useSubmissions(assignmentId: string | null) {
  return useQuery<SubmissionRow[]>(assignmentId ? `/assignments/${assignmentId}/submissions` : null, () => fetchAll<SubmissionRow>(`/assignments/${assignmentId}/submissions`));
}

export function useStudentAttempts(assignmentId: string | null, studentId: string | null) {
  return useQuery<Attempt[]>(assignmentId && studentId ? `/assignments/${assignmentId}/submissions/${studentId}` : null, () =>
    fetchAll<Attempt>(`/assignments/${assignmentId}/submissions/${studentId}`),
  );
}

export function useGradebook(courseId: string | null) {
  return useQuery<GradebookPage>(courseId ? `/courses/${courseId}/gradebook` : null, async () => {
    // The gradebook pages by student; the columns come with every page.
    const first = await api.get<GradebookPage>(`/courses/${courseId}/gradebook?limit=100`);
    let cursor = first.next_cursor;
    const items = [...first.items];
    while (cursor) {
      const page: GradebookPage = await api.get<GradebookPage>(`/courses/${courseId}/gradebook?limit=100&cursor=${encodeURIComponent(cursor)}`);
      items.push(...page.items);
      cursor = page.next_cursor;
    }
    return { columns: first.columns, items, next_cursor: null };
  });
}

/** The recorded changes of one grade, newest first (ADR-0049). The drawer shows the latest ones. */
export function useGradeHistory(assignmentId: string | null, studentId: string | null) {
  const path = `/assignments/${assignmentId}/grades/${studentId}/history?limit=20`;
  return useQuery<GradeHistoryEntry[]>(assignmentId && studentId ? path : null, async () => (await api.get<{ items: GradeHistoryEntry[] }>(path)).items);
}

export function useSaveGrade(assignmentId: string) {
  return useMutation(async (studentId: string, score: string | null, feedback?: string | null) => {
    const cell = await api.put<unknown>(`/assignments/${assignmentId}/grades/${studentId}`, { score, feedback: feedback ?? null });
    invalidate("/courses/");
    invalidate("/me/");
    invalidate(`/assignments/${assignmentId}`);
    return cell;
  });
}

export function usePublishGrades(assignmentId: string) {
  return useMutation(async (students: "all" | string[]) => {
    const r = await api.post<{ published: number }>(`/assignments/${assignmentId}/grades/publish`, { students });
    invalidate("/courses/");
    invalidate("/me/");
    invalidate(`/assignments/${assignmentId}`);
    return r;
  });
}

/** The responses to a form (ADR-0051), as the fixed columns of the responses table. */
export function useFormResponses(assignmentId: string | null, config: FormConfig) {
  return useQuery<FormResponses>(assignmentId ? `/assignments/${assignmentId}/responses` : null, async () => formResponsesFromWire(await api.get(`/assignments/${assignmentId}/responses`), config));
}

/** The latest submission of every student for several assignments, keyed by assignment id (the gradebook needs "to grade"). */
export function useCourseSubmissions(courseId: string | null, assignmentIds: string[]) {
  const key = courseId ? `/courses/${courseId}/submissions-overview?ids=${assignmentIds.join(",")}` : null;
  return useQuery<Record<string, SubmissionRow[]>>(assignmentIds.length ? key : null, async () => {
    const entries = await Promise.all(assignmentIds.map(async (id) => [id, await fetchAll<SubmissionRow>(`/assignments/${id}/submissions`)] as const));
    return Object.fromEntries(entries);
  });
}

/** Publishes every draft grade of the given assignments, for all students. */
export function usePublishAllDrafts() {
  return useMutation(async (assignmentIds: string[]) => {
    let published = 0;
    for (const id of assignmentIds) published += (await api.post<{ published: number }>(`/assignments/${id}/grades/publish`, { students: "all" })).published;
    invalidate("/courses/");
    invalidate("/me/");
    return published;
  });
}
