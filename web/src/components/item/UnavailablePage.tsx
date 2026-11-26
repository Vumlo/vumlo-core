"use client";

import { Icon } from "@/components/Icon";
import type { Item } from "@/lib/api/types";
import { useT } from "@/lib/i18n";
import { ItemFrame } from "./ItemFrame";
import s from "./Item.module.css";

/** An item of a type that no enabled module provides (ADR-0098): the title and a notice; the server sends no body. */
export function UnavailablePage({ item, courseId }: { item: Item; courseId: string }) {
  const t = useT();
  return (
    <ItemFrame courseId={courseId} itemId={item.id}>
      {({ crumbs }) => (
        <div className={s.lecture}>
          <article className={s.article}>
            {crumbs}
            <h1 className={s.lectureTitle}>{item.title}</h1>
            <div className="card card-pad" role="status">
              <h2 className="card-title">
                <Icon name="lock" size={18} strokeWidth={2} /> {t("unavailable.title")}
              </h2>
              <p>{t("unavailable.lead")}</p>
            </div>
          </article>
        </div>
      )}
    </ItemFrame>
  );
}
