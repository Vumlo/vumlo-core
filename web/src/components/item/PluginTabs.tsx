"use client";

import Link from "next/link";
import type { PluginSlot } from "@/lib/api/types";
import { useT } from "@/lib/i18n";
import { itemHref } from "@/lib/routes";
import s from "./Item.module.css";

/** "Assignment" and one tab for each plugin that offers a page here (ADR-0066). The open tab is in the address. */
export function PluginTabs({ courseId, itemId, slots, current }: { courseId: string; itemId: string; slots: PluginSlot[]; current: string }) {
  const t = useT();
  return (
    <nav className={`tabs ${s.pluginTabs}`} aria-label={t("plugin.tabs")}>
      <Link href={itemHref(courseId, itemId)} aria-current={current ? undefined : "page"}>{t("plugin.tab.assignment")}</Link>
      {slots.map((slot) => (
        <Link key={slot.plugin_id} href={itemHref(courseId, itemId, slot.plugin_id)} aria-current={current === slot.plugin_id ? "page" : undefined}>
          {slot.title}
        </Link>
      ))}
    </nav>
  );
}
