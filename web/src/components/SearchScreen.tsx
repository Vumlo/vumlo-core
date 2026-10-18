"use client";

import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { useState, type FormEvent } from "react";
import { Icon } from "@/components/Icon";
import { ItemTile } from "@/components/ItemTile";
import { useSearch } from "@/lib/api/hooks/work";
import type { SearchHit } from "@/lib/api/types";
import { dueLabel } from "@/lib/format";
import { useT } from "@/lib/i18n";
import { useMe } from "@/lib/session";
import s from "./Search.module.css";

type Filter = "all" | SearchHit["kind"];
const FILTERS: Filter[] = ["all", "course", "lecture", "assignment"];

/** The matching part of `text`, marked. Plain string search, so the query never acts as a pattern. */
function Marked({ text, q }: { text: string; q: string }) {
  const i = q ? text.toLowerCase().indexOf(q.toLowerCase()) : -1;
  if (i < 0) return <>{text}</>;
  return (
    <>
      {text.slice(0, i)}
      <mark>{text.slice(i, i + q.length)}</mark>
      {text.slice(i + q.length)}
    </>
  );
}

function href(h: SearchHit, staff: boolean) {
  return h.kind === "course" ? `/courses/${h.id}` : `/courses/${h.course_id}/items/${h.id}${staff && h.status === "draft" ? "/edit" : ""}`;
}

export function SearchScreen() {
  const t = useT();
  const router = useRouter();
  const q = (useSearchParams().get("q") ?? "").trim();
  const me = useMe();
  const results = useSearch(q);
  const [filter, setFilter] = useState<Filter>("all");
  const [draft, setDraft] = useState<{ for: string; value: string }>({ for: q, value: q });
  // The field follows the address (a search from the navbar), until the person types in it.
  const value = draft.for === q ? draft.value : q;
  const staff = me.data?.role !== "student";

  function submit(e: FormEvent) {
    e.preventDefault();
    const next = value.trim();
    router.push(next ? `/search?q=${encodeURIComponent(next)}` : "/search");
  }

  const all = results.data?.items ?? [];
  const count = (f: Filter) => (f === "all" ? all.length : all.filter((h) => h.kind === f).length);
  const shown = filter === "all" ? all : all.filter((h) => h.kind === filter);

  return (
    <div className="container">
      <h1 className="page-title" style={{ marginBottom: 16 }}>{t("search.title")}</h1>
      <form role="search" onSubmit={submit} className={s.bar}>
        <label className={`search ${s.field}`}>
          <Icon name="search" size={18} />
          <input type="search" aria-label={t("search.label")} placeholder={t("search.placeholder")} value={value} onChange={(e) => setDraft({ for: q, value: e.target.value })} autoFocus />
        </label>
        <button type="submit" className="btn btn-primary">{t("search.submit")}</button>
      </form>

      {q.length < 2 ? (
        <p className="empty">{q.length === 0 ? t("search.prompt") : t("search.tooShort")}</p>
      ) : results.error ? (
        <p className="error-box" role="alert">{t("search.failed")}</p>
      ) : !results.data ? (
        <p className="loading" role="status">{t("search.searching")}</p>
      ) : (
        <>
          <div role="tablist" aria-label={t("search.filter")} className={`segmented ${s.filters}`}>
            {FILTERS.map((f) => (
              <button key={f} type="button" role="tab" aria-selected={filter === f} onClick={() => setFilter(f)}>
                {t(`search.filter.${f}`)}, {count(f)}
              </button>
            ))}
          </div>
          <p className="muted" role="status" style={{ margin: "0 2px 12px" }}>{t.plural("search.results", shown.length, { q })}</p>
          {shown.length === 0 ? (
            <p className="empty">{t("search.none")}</p>
          ) : (
            <ul className={`card card-list ${s.list}`}>
              {shown.map((h) => (
                <li key={`${h.kind}:${h.id}`}>
                  <Link href={href(h, staff)} className={s.hit}>
                    {h.kind === "course" ? (
                      <span aria-hidden="true" className={`tile ${s.courseTile}`}><Icon name="courses" size={19} /></span>
                    ) : (
                      <ItemTile kind={h.kind} />
                    )}
                    <span className={s.body}>
                      <span className={s.title}>
                        <Marked text={h.title} q={q} />
                        {h.status === "draft" && <span className="pill pill-dashed" style={{ marginLeft: 8 }}>{t("common.draft")}</span>}
                      </span>
                      <span className={s.meta}>
                        {h.kind === "course" ? t("search.kind.course") : `${h.course_code ? `${h.course_code}, ` : ""}${h.course_title}`}
                        {h.due_at && `, ${t("search.due", { when: dueLabel(h.due_at) })}`}
                      </span>
                      {h.snippet && <span className={s.snippet}><Marked text={h.snippet} q={q} /></span>}
                    </span>
                  </Link>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </div>
  );
}
