"use client";

import type { Item } from "@/lib/api/types";
import { SheetPage } from "./SheetPage";

export function QuizPage(props: { item: Item; courseId: string; staff: boolean }) {
  return <SheetPage {...props} mode="quiz" />;
}
