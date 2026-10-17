"use client";

import { useParams } from "next/navigation";
import { People } from "@/components/course/People";

export default function PeoplePage() {
  const { courseId } = useParams<{ courseId: string }>();
  return <People courseId={courseId} />;
}
