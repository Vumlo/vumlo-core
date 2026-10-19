"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { BlockEditor } from "@/components/doc/BlockEditor";
import { typeLabel } from "@/lib/course";
import { useEditorProfiles, usePublishItem, useUpdateItem } from "@/lib/api/hooks/content";
import { useAutosave } from "@/lib/autosave";
import { bodyOf, type Document, type Feature, type Item } from "@/lib/api/types";
import { AssignmentSettings } from "./AssignmentSettings";
import { AuthorBar } from "./AuthorBar";
import { useT } from "@/lib/i18n";
import s from "./Author.module.css";
import { itemHref } from "@/lib/routes";

const FULL: Feature[] = ["headings", "text_styles", "links", "lists", "quote", "divider", "table", "code", "math", "mermaid", "video"];

/** Title, instructions and settings of a text or in-class assignment. */
export function AssignmentEditor({ item, courseId }: { item: Item; courseId: string }) {
  const t = useT();
  const profiles = useEditorProfiles();
  const update = useUpdateItem(courseId);
  const publish = usePublishItem(courseId);
  const [initial] = useState(() => bodyOf(item));
  const [doc, setDoc] = useState<Document>(initial);
  const [title, setTitle] = useState(item.title);
  const draft = useMemo(() => ({ doc, title }), [doc, title]);
  const auto = useAutosave(draft, (d) => update.run(item.id, { title: d.title.trim() || item.title, body: d.doc }));
  const features = profiles.data?.profiles.find((p) => p.name === "full")?.features ?? FULL;
  const isDraft = item.status === "draft";

  return (
    <div className="container" style={{ paddingTop: 20 }}>
      <AuthorBar courseId={courseId} itemId={item.id} title={title} chip={isDraft ? { text: t("le.draftChip"), tone: "draft" } : { text: t("cm.published"), tone: "ok" }} saved={auto.savedAt} pending={auto.pending} failed={auto.failed}>
        <Link href={itemHref(courseId, item.id)} className="btn">{t("cm.preview")}</Link>
        {isDraft && <button type="button" className="btn btn-primary" disabled={publish.pending} onClick={async () => { await auto.flush(); await publish.run(item.id); }}>{t("cm.publish")}</button>}
      </AuthorBar>
      <div className={s.shell}>
        <div className={s.canvas}>
          <div className={s.sheet}>
            <p className="muted" style={{ fontSize: 14, marginBottom: 10 }}><strong style={{ color: "var(--asg-fg)" }}>{t("cm.assignment")}</strong> {typeLabel(item.assignment!.type)}</p>
            <label htmlFor="title" className="sr-only">{t("cm.title")}</label>
            <input id="title" className={s.titleInput} value={title} onChange={(e) => setTitle(e.target.value)} />
            <h2 style={{ fontSize: 16, fontWeight: 600, margin: "20px 0 8px" }}>{t("item.instructions")}</h2>
            <div style={{ paddingLeft: 34 }}>
              <BlockEditor value={initial} onChange={setDoc} features={features} label={t("item.instructions")} mode="float" minHeight={260} />
            </div>
          </div>
        </div>
        <aside aria-label={t("as.title")} className={`card ${s.panel}`}>
          <div className={s.panelBody}>
            <h2 style={{ fontSize: 16, fontWeight: 600 }}>{t("as.title")}</h2>
            <AssignmentSettings item={item} courseId={courseId} />
          </div>
        </aside>
      </div>
    </div>
  );
}
