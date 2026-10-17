"use client";

import { api } from "../client";
import { invalidate, useMutation, useQuery } from "../../query";
import type { AnswerMap, FormResponses, GradeHistory, QuizResult } from "../gaps";
import type { Attempt, Document, GradebookPage, MySubmission, SubmissionRow } from "../types";
import { fetchAll } from "./util";

export function useMySubmission(assignmentId: string | null) {
  return useQuery<MySubmission>(assignmentId ? `/assignments/${assignmentId}/submission` : null, () => api.get<MySubmission>(`/assignments/${assignmentId}/submission`));
}

/** Quizzes and forms store their answers in the draft's `content` as `{answers}` (API_GAPS.md, "Quiz and form types"). */
export type DraftContent = Document | { answers: AnswerMap };

export function useSaveDraft(assignmentId: string) {
  return useMutation(async (content: DraftContent) => {
    const d = await api.put<{ content: DraftContent; updated_at: string }>(`/assignments/${assignmentId}/submission/draft`, { content });
    invalidate(`/assignments/${assignmentId}/submission`);
    invalidate("/me/work");
    return d;
  });
}

export function useSubmitDraft(assignmentId: string) {
  return useMutation(async () => {
    const a = await api.post<Attempt>(`/assignments/${assignmentId}/submission/submit`);
    invalidate(`/assignments/${assignmentId}`);
    invalidate("/me/");
    invalidate("/courses/");
    return a;
  });
}

/** GAP: per-question result of a graded quiz (API_GAPS.md, "Quiz and form types"). */
export function useQuizResult(assignmentId: string | null) {
  return useQuery<QuizResult>(assignmentId ? `/assignments/${assignmentId}/result` : null, () => api.get<QuizResult>(`/assignments/${assignmentId}/result`));
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

/** GAP: the change history of one grade (API_GAPS.md, "Grade history"). */
export function useGradeHistory(assignmentId: string | null, studentId: string | null) {
  return useQuery<GradeHistory>(assignmentId && studentId ? `/assignments/${assignmentId}/grades/${studentId}/history` : null, () =>
    api.get<GradeHistory>(`/assignments/${assignmentId}/grades/${studentId}/history`),
  );
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

/** GAP: the responses to a form (API_GAPS.md, "Quiz and form types"). */
export function useFormResponses(assignmentId: string | null) {
  return useQuery<FormResponses>(assignmentId ? `/assignments/${assignmentId}/responses` : null, () => api.get<FormResponses>(`/assignments/${assignmentId}/responses`));
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
