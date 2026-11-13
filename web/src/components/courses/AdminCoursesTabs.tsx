"use client";

import { useState } from "react";
import { useT } from "@/lib/i18n";
import { AllCourses } from "./AllCourses";
import { TeacherCourses } from "./TeacherCourses";

/** The courses page of an administrator: their own courses, or every course of the institution. */
export function AdminCoursesTabs() {
  const t = useT();
  const [tab, setTab] = useState<"mine" | "all">("mine");
  return (
    <>
      <div role="tablist" aria-label={t("courses.which")} className="segmented" style={{ marginBottom: 16 }}>
        <button type="button" role="tab" aria-selected={tab === "mine"} onClick={() => setTab("mine")}>{t("courses.mine")}</button>
        <button type="button" role="tab" aria-selected={tab === "all"} onClick={() => setTab("all")}>{t("courses.all")}</button>
      </div>
      {tab === "mine" ? <TeacherCourses /> : <AllCourses />}
    </>
  );
}
