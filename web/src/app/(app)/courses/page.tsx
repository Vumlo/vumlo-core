"use client";

import { Suspense } from "react";
import { StudentCourses } from "@/components/courses/StudentCourses";
import { TeacherCourses } from "@/components/courses/TeacherCourses";
import { useMe } from "@/lib/session";

function Courses() {
  const me = useMe();
  if (!me.data) return null;
  return me.data.role === "student" ? <StudentCourses /> : <TeacherCourses />;
}

export default function CoursesPage() {
  return (
    <Suspense fallback={null}>
      <Courses />
    </Suspense>
  );
}
