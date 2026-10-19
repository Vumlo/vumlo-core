"use client";

import { People } from "@/components/course/People";
import { useCourseId } from "@/lib/useRoute";

export default function PeoplePage() {
  return <People courseId={useCourseId()} />;
}
