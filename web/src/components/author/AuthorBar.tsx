"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { useContentTree, useCourseInfo } from "@/lib/api/hooks/courses";
import { findItem } from "@/lib/course";
import { timeOf } from "@/lib/format";
import s from "./Author.module.css";

export function Switch({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return <button type="button" role="switch" aria-checked={checked} aria-label={label} className={s.sw} onClick={() => onChange(!checked)} />;
}

/** Breadcrumb, status chip, save state and the main actions of an authoring screen. */
export function AuthorBar({ courseId, itemId, title, chip, saved, pending, failed, children }: {
  courseId: string;
  itemId: string;
  title: string;
  chip: { text: string; tone: "accent" | "ok" | "draft" };
  saved: string | null;
  pending: boolean;
  failed: boolean;
  children?: ReactNode;
}) {
  const info = useCourseInfo(courseId);
  const tree = useContentTree(courseId);
  const found = tree.data ? findItem(tree.data, itemId) : undefined;
  return (
    <div className={s.top}>
      <nav aria-label="Breadcrumb" className={`crumbs ${s.crumbs}`}>
        <Link href={`/courses/${courseId}`}>{info.data?.code ?? "Course"}</Link>
        <span aria-hidden="true" style={{ padding: "0 6px" }}>/</span>
        <Link href={`/courses/${courseId}`}>{found?.module.title ?? "…"}</Link>
        <span aria-hidden="true" style={{ padding: "0 6px" }}>/</span>
        <span aria-current="page">{title || "Untitled"}</span>
      </nav>
      <span className={s.chip} data-tone={chip.tone}>{chip.text}</span>
      <span role="status" className={s.saved} style={failed ? { color: "var(--danger)" } : undefined}>{failed ? "Could not save" : pending ? "Saving…" : saved ? `Saved ${timeOf(saved)}` : "All changes saved"}</span>
      {children}
    </div>
  );
}
