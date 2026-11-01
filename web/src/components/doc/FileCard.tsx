"use client";

import { useState, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import { errorText } from "@/lib/api/client";
import { useFileInfo } from "@/lib/api/hooks/files";
import { fileUrl, openFile, sizeLabel } from "@/lib/files";
import { useT } from "@/lib/i18n";
import { notice } from "@/lib/notice";
import s from "./Doc.module.css";

/** The picture of an `image` block. Bytes that do not load (a file that is no picture, or one that is gone) show a neutral box. */
export function Picture({ fileId, alt }: { fileId: string; alt: string }) {
  const t = useT();
  const [failed, setFailed] = useState(false);
  if (failed || !fileId) return <span role="img" aria-label={alt || t("be.imageFailed")} className={s.pictureFailed}><Icon name="file" size={20} strokeWidth={2} />{t("be.imageFailed")}</span>;
  // eslint-disable-next-line @next/next/no-img-element -- the address is the API's (or the desktop's) file route, not a static asset
  return <img className={s.picture} src={fileUrl(fileId)} alt={alt} onError={() => setFailed(true)} />;
}

/** The card of a `file` block: icon, name, size, what opening does, and a button that opens it. `children` are further buttons. */
export function FileCard({ fileId, children }: { fileId: string; children?: ReactNode }) {
  const t = useT();
  const info = useFileInfo(fileId || null);
  const file = info.data;
  return (
    <div role="group" aria-label={t("files.card")} className={s.fileCard}>
      <span aria-hidden="true" className={s.fileIcon}><Icon name="file" size={22} strokeWidth={2} /></span>
      <span className={s.fileMain}>
        <span className={s.fileName}>{file ? file.name || t("blk.file") : t("blk.file")}</span>
        <span className={s.fileMeta}>
          {file ? `${sizeLabel(file.size)}, ${t(file.mime === "application/pdf" ? "files.opensNewTab" : "files.savesToDevice")}` : info.error ? errorText(info.error) : t("common.loading")}
        </span>
      </span>
      {file && <button type="button" className="btn btn-primary" onClick={() => void openFile(file).catch((e: unknown) => notice(e instanceof Error ? e.message : String(e), "error"))}>{t("files.open")}</button>}
      {children}
    </div>
  );
}
