"use client";

import Link from "next/link";
import { useMemo, useRef, useState } from "react";
import { BlockEditor } from "@/components/doc/BlockEditor";
import { Icon } from "@/components/Icon";
import { FileCard } from "@/components/item/FilePage";
import { ApiError, errorText } from "@/lib/api/client";
import { useItemRevision, useEditorProfiles, usePublishItem, usePublishRevision, useUpdateItem } from "@/lib/api/hooks/content";
import { useFileInfo } from "@/lib/api/hooks/files";
import { emptyDocument, fileBodyOf, fileBodyToWire, type Document, type Feature, type FileRef, type Item } from "@/lib/api/types";
import { useAutosave } from "@/lib/autosave";
import { uploadFile } from "@/lib/files";
import { useT } from "@/lib/i18n";
import { itemHref } from "@/lib/routes";
import { AuthorBar } from "./AuthorBar";
import s from "./Author.module.css";

const FULL: Feature[] = ["headings", "text_styles", "links", "lists", "quote", "divider", "table", "code", "math", "mermaid", "video"];

/**
 * The editor of a `file` item: a title, one file and a description. The file is uploaded for the item as soon as it is
 * chosen and named by the body that autosave then stores; removing it leaves `file_id` out of the body.
 */
export function FileItemEditor({ item, courseId }: { item: Item; courseId: string }) {
  const t = useT();
  const profiles = useEditorProfiles();
  const revision = useItemRevision(item.id);
  const update = useUpdateItem(courseId);
  const publishItem = usePublishItem(courseId);
  const publishRevision = usePublishRevision(item.id, courseId);
  const input = useRef<HTMLInputElement>(null);
  const [initial] = useState(() => fileBodyOf(item));
  const target = useMemo(() => ({ item: item.id }), [item.id]);
  const [initialDescription] = useState<Document>(() => initial.description ?? emptyDocument());
  const [description, setDescription] = useState(initialDescription);
  const [title, setTitle] = useState(item.title);
  const [fileId, setFileId] = useState<string | null>(initial.file_id ?? null);
  // The file just uploaded is known in full; a stored one is read from the server.
  const [uploaded, setUploaded] = useState<FileRef | null>(null);
  const [busy, setBusy] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);
  const stored = useFileInfo(fileId && uploaded?.id !== fileId ? fileId : null);
  const file = uploaded && uploaded.id === fileId ? uploaded : stored.data;
  const features = profiles.data?.profiles.find((p) => p.name === "full")?.features ?? FULL;

  const draft = useMemo(() => ({ title, description, fileId }), [title, description, fileId]);
  const auto = useAutosave(draft, (d) => update.run(item.id, { title: d.title.trim() || item.title, body: fileBodyToWire(d.fileId, d.description) }));
  const rev = revision.data;
  const isDraft = item.status === "draft";
  const changed = !isDraft && !!rev?.has_unpublished_changes;
  const chip = isDraft ? { text: t("le.draftChip"), tone: "draft" as const } : changed ? { text: t("le.unpublishedChip"), tone: "accent" as const } : { text: t("cm.published"), tone: "ok" as const };
  // A file that the server no longer has (an upload nobody named for a day is swept) is refused with this field.
  const gone = update.error?.errors.some((e) => e.field.startsWith("body.file_id")) ?? false;
  const error = gone ? t("files.reupload") : uploadError ?? errorText(publishItem.error ?? publishRevision.error ?? update.error);

  async function choose(f: File | undefined) {
    if (!f) return;
    setBusy(true);
    setUploadError(null);
    try {
      const ref = await uploadFile(f, { item: item.id });
      setUploaded(ref);
      setFileId(ref.id);
    } catch (e) {
      setUploadError(errorText(e instanceof ApiError ? e : undefined) ?? t("files.uploadFailed"));
    } finally {
      setBusy(false);
      if (input.current) input.current.value = "";
    }
  }
  async function publish() {
    if (!(await auto.flush())) return;
    if (isDraft) await publishItem.run(item.id);
    else await publishRevision.run();
  }
  const canPublish = (isDraft || changed) && !!fileId;

  return (
    <div className="container" style={{ paddingTop: 20 }}>
      <AuthorBar courseId={courseId} itemId={item.id} title={title} chip={chip} saved={auto.savedAt} pending={auto.pending} failed={auto.failed} error={error}>
        <Link href={itemHref(courseId, item.id)} className="btn">{t("cm.preview")}</Link>
        <button type="button" className="btn btn-primary" disabled={!canPublish || publishItem.pending || publishRevision.pending} onClick={publish}>{isDraft ? t("cm.publish") : t("le.publishChanges")}</button>
      </AuthorBar>
      <div className={s.canvas} style={{ maxWidth: 820 }}>
        <div className={s.sheet}>
          <div className="row" style={{ gap: 8, marginBottom: 12 }}>
            <span aria-hidden="true" style={{ display: "flex", alignItems: "center", justifyContent: "center", width: 26, height: 26, borderRadius: 7, background: "var(--file-tile)", color: "var(--file-fg)" }}><Icon name="file" size={15} strokeWidth={2} /></span>
            <span style={{ fontSize: 14, fontWeight: 600, color: "var(--file-fg)" }}>{t("row.file")}</span>
          </div>
          <label htmlFor="title" className="sr-only">{t("cm.title")}</label>
          <input id="title" className={s.titleInput} value={title} onChange={(e) => setTitle(e.target.value)} aria-invalid={!title.trim()} />
          {!title.trim() && <p className="hint" style={{ color: "var(--danger)", marginTop: 8 }}>{t("le.needsTitle")}</p>}

          <input ref={input} type="file" hidden onChange={(e) => void choose(e.target.files?.[0])} />
          <h2 className="card-title" style={{ marginTop: 24 }}>{t("files.card")}</h2>
          {file ? (
            <FileCard file={file} title={title}>
              <span className="row" style={{ gap: 8 }}>
                <button type="button" className="btn" disabled={busy} onClick={() => input.current?.click()}>{busy ? t("fe.uploading") : t("fe.replace")}</button>
                <button type="button" className="btn" disabled={busy} onClick={() => setFileId(null)}>{t("fe.remove")}</button>
              </span>
            </FileCard>
          ) : fileId ? (
            stored.error ? <p role="alert" className="alert alert-danger">{errorText(stored.error)}</p> : <p className="loading" role="status">{t("common.loading")}</p>
          ) : (
            <div className="row" style={{ gap: 12, flexWrap: "wrap" }}>
              <button type="button" className="btn btn-outline" disabled={busy} onClick={() => input.current?.click()}><Icon name="plus" size={15} strokeWidth={2.2} />{busy ? t("fe.uploading") : t("fe.choose")}</button>
              <span className="muted" style={{ fontSize: 14 }}>{t("fe.noFile")}</span>
            </div>
          )}
          {!fileId && <p className="hint" style={{ marginTop: 8 }}>{t("fe.needsFile")}</p>}

          <h2 className="card-title" style={{ marginTop: 24 }}>{t("fe.description")}</h2>
          <p className="muted" style={{ margin: "0 0 8px", fontSize: 14 }}>{t("fe.descriptionHint")}</p>
          <BlockEditor value={initialDescription} onChange={setDescription} features={features} label={t("fe.description")} mode="float" minHeight={160} uploadTarget={target} errors={update.error?.errors} errorPath="body.description" />
        </div>
      </div>
    </div>
  );
}
