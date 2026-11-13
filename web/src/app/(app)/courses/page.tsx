"use client";

import { Suspense } from "react";
import { AdminCoursesTabs } from "@/components/courses/AdminCoursesTabs";
import { StudentCourses } from "@/components/courses/StudentCourses";
import { TeacherCourses } from "@/components/courses/TeacherCourses";
import { useMe } from "@/lib/session";

function Courses() {
  const me = useMe();
  if (!me.data) return null;
  if (me.data.role === "admin") return <AdminCoursesTabs />;
  return me.data.role === "student" ? <StudentCourses /> : <TeacherCourses />;
}

export default function CoursesPage() {
  return (
    <Suspense fallback={null}>
      <Courses />
    </Suspense>
  );
}
