"use client";

import { useRouter } from "next/navigation";
import { useEffect } from "react";
import { GradingScreen } from "@/components/GradingScreen";
import { useMe } from "@/lib/session";

export default function GradingPage() {
  const me = useMe();
  const router = useRouter();
  const role = me.data?.role;
  useEffect(() => {
    if (role === "student") router.replace("/grades");
  }, [role, router]);
  return role && role !== "student" ? <GradingScreen /> : null;
}
