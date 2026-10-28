"use client";

import { GradeWorkspace } from "@/components/grading/GradeWorkspace";
import { useCourseId, useItemId, useStudentId } from "@/lib/useRoute";

export default function GradePage() {
  return <GradeWorkspace courseId={useCourseId()} assignmentId={useItemId()} studentId={useStudentId()} />;
}
