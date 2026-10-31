"use client";

import { useMemo, type ReactNode } from "react";
import { BlockRenderer } from "@/components/doc/BlockRenderer";
import { Icon } from "@/components/Icon";
import { useCourseInfo } from "@/lib/api/hooks/courses";
import { useFileInfo } from "@/lib/api/hooks/files";
import { errorText } from "@/lib/api/client";
import { emptyDocument, fileBodyOf, type FileRef, type Item } from "@/lib/api/types";
import { dayLong } from "@/lib/format";
import { openFile, sizeLabel } from "@/lib/files";
import { useT } from "@/lib/i18n";
import { notice } from "@/lib/notice";
import { ItemFrame, PreviewBanner } from "./ItemFrame";
import s from "./Item.module.css";

/** A stored file as a card: icon, name, size and what opening it does. `children` are its buttons. */
export function FileCard({ file, title, children }: { file: FileRef; title: string; children?: ReactNode }) {
  const t = useT();
  return (
    <div role="group" aria-label={t("files.card")} className={s.fileCard}>
      <span aria-hidden="true" className={s.fileIcon}><Icon name="file" size={22} strokeWidth={2} /></span>
      <span className={s.fileMain}>
        <span className={s.fileName}>{file.name || title}</span>
        <span className={s.fileMeta}>{sizeLabel(file.size)}, {t(file.mime === "application/pdf" ? "files.opensNewTab" : "files.savesToDevice")}</span>
      </span>
      {children}
    </div>
  );
}

/** The page of a `file` item: what the teacher wrote about the file, and the file to open. */
export function FilePage({ item, courseId, staff }: { item: Item; courseId: string; staff: boolean }) {
  const t = useT();
  const info = useCourseInfo(courseId);
  const body = useMemo(() => fileBodyOf(item), [item]);
  const file = useFileInfo(body.file_id ?? null);
  const description = body.description ?? emptyDocument();
  return (
    <ItemFrame courseId={courseId} itemId={item.id}>
      {({ crumbs }) => (
        <div className={s.lecture}>
          <article className={s.article}>
            {staff && <PreviewBanner courseId={courseId} itemId={item.id} />}
            {crumbs}
            <div className={s.kind}>
              <span aria-hidden="true" className={s.kindIcon} data-kind="file"><Icon name="file" size={15} strokeWidth={2} /></span>
              <span className={s.kindLabel} data-kind="file">{t("row.file")}</span>
            </div>
            <h1 className={s.lectureTitle}>{item.title}</h1>
            <p className={s.byline}>{info.data?.owner_name}, {t("lecture.updated", { day: dayLong(item.updated_at) })}</p>
            {description.blocks.length > 0 && <BlockRenderer doc={description} />}
            {file.data ? (
              <FileCard file={file.data} title={item.title}>
                <button type="button" className="btn btn-primary" onClick={() => void openFile(file.data!).catch((e: unknown) => notice(e instanceof Error ? e.message : String(e)))}>{t("files.open")}</button>
              </FileCard>
            ) : file.error ? (
              <p role="alert" className="alert alert-danger" style={{ marginTop: 24 }}>{errorText(file.error)}</p>
            ) : body.file_id ? (
              <p className="loading" role="status">{t("common.loading")}</p>
            ) : (
              <p className="muted" style={{ marginTop: 24 }}>{t("fe.noFile")}</p>
            )}
          </article>
        </div>
      )}
    </ItemFrame>
  );
}
