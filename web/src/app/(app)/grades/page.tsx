"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { StudentGrades } from "@/components/StudentGrades";
import { useMe } from "@/lib/session";

export default function GradesPage() {
  const me = useMe();
  const router = useRouter();
  const role = me.data?.role;
  useEffect(() => {
    if (role && role !== "student") router.replace("/grading");
  }, [role, router]);
  return role === "student" ? <StudentGrades /> : null;
}
