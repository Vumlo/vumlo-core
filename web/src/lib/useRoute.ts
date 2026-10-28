"use client";

import { useSearchParams } from "next/navigation";

/** The course of a `/course/...` screen. The course layout shows "not found" when it is missing, so a screen below it can rely on it. */
export function useCourseId() {
  return useSearchParams().get("id") ?? "";
}

/** The item of a `/course/item...` screen; empty when the address names none. */
export function useItemId() {
  return useSearchParams().get("item") ?? "";
}

/** The student whose work is open in the grading workspace, `&student=<user id>`; empty when none. */
export function useStudentId() {
  return useSearchParams().get("student") ?? "";
}

/** The plugin whose tab is open on an assignment, `&tab=<plugin id>`; empty for the assignment itself. */
export function useItemTab() {
  return useSearchParams().get("tab") ?? "";
}

/** The segments of a module page address, `/m?name=x&path=a/b`. */
export function useModuleAddress() {
  const params = useSearchParams();
  const path = params.get("path");
  return { name: params.get("name") ?? "", path: path ? path.split("/").filter(Boolean) : [] };
}
