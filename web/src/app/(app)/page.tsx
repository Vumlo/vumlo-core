"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { StudentHome } from "@/components/home/StudentHome";
import { TeacherHome } from "@/components/home/TeacherHome";
import { useMe } from "@/lib/session";

export default function Home() {
  const me = useMe();
  const router = useRouter();
  const role = me.data?.role;

  useEffect(() => {
    if (role === "admin") router.replace("/admin");
  }, [role, router]);

  if (!me.data || role === "admin") return null;
  return role === "teacher" ? <TeacherHome user={me.data} /> : <StudentHome user={me.data} />;
}
