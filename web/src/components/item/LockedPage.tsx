"use client";

import { Icon } from "@/components/Icon";
import type { Item } from "@/lib/api/types";
import { unmetText } from "@/lib/course";
import { useT } from "@/lib/i18n";
import { ItemFrame } from "./ItemFrame";
import s from "./Item.module.css";

/** What a student sees of an item that a rule still keeps closed (ADR-0092): its title and what to do first. */
export function LockedPage({ item, courseId }: { item: Item; courseId: string }) {
  const t = useT();
  const missing = item.standing?.locked ?? [];
  return (
    <ItemFrame courseId={courseId} itemId={item.id}>
      {({ crumbs }) => (
        <div className={s.lecture}>
          <article className={s.article}>
            {crumbs}
            <h1 className={s.lectureTitle}>{item.title}</h1>
            <div className="card card-pad" role="status">
              <h2 className="card-title">
                <Icon name="lock" size={18} strokeWidth={2} /> {t("locked.title")}
              </h2>
              <p>{t("locked.lead")}</p>
              <ul>
                {missing.map((unmet, index) => (
                  <li key={index}>{unmetText(unmet)}</li>
                ))}
              </ul>
            </div>
          </article>
        </div>
      )}
    </ItemFrame>
  );
}
