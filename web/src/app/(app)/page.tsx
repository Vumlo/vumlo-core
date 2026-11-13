"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { AppIsland } from "@/components/app/AppIsland";
import { StudentHome } from "@/components/home/StudentHome";
import { TeacherHome } from "@/components/home/TeacherHome";
import { useMe } from "@/lib/session";

export default function Home() {
  const me = useMe();
  const router = useRouter();
  const role = me.data?.role;

  useEffect(() => {
    // An administrator's home is the list of every course; administration is the console's (ADR-0095).
    if (role === "admin") router.replace("/courses");
  }, [role, router]);

  if (!me.data || role === "admin") return null;
  return (
    <>
      <AppIsland />
      {role === "teacher" ? <TeacherHome user={me.data} /> : <StudentHome user={me.data} />}
    </>
  );
}
