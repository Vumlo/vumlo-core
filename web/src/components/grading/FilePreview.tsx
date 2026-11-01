"use client";

import { useState } from "react";
import { BlockRenderer } from "@/components/doc/BlockRenderer";
import { Icon } from "@/components/Icon";
import { FileCard } from "@/components/item/FilePage";
import { errorText, isDesktop } from "@/lib/api/client";
import { useFileInfo } from "@/lib/api/hooks/files";
import { useQuery } from "@/lib/query";
import type { FileRef } from "@/lib/api/types";
import { fileContentOf, isBlankDoc } from "@/lib/fileAssignment";
import { fileUrl, openFile, readFileText, sizeLabel } from "@/lib/files";
import { useT } from "@/lib/i18n";
import { notice } from "@/lib/notice";
import s from "./FilePreview.module.css";

/**
 * Whether a PDF is drawn in an <iframe> while grading. Off: the server sends every file with `Content-Security-Policy: sandbox`,
 * and Chrome does not draw a PDF in a sandboxed frame (STEP 0 of Task 14 could not be run to check it, so the safe card is used).
 * Turn it on when the server serves `application/pdf` without that policy; the iframe code and its spec are in place.
 */
export const PDF_IFRAME_PREVIEW = false;

/** The flag as the screens read it. Nothing but the mock build's test hook (src/mocks/StartWorker.tsx) ever changes it. */
export const pdfPreview = { iframe: PDF_IFRAME_PREVIEW };

/** What the text preview reads of a file. */
const TEXT_MAX = 1_048_576;
// The types the server serves `inline` (root AGENTS.md section 6), minus the PDF, which has its own case. SVG is never among them.
const IMAGES = new Set(["image/png", "image/jpeg", "image/gif", "image/webp"]);

export type Preview = "image" | "text" | "pdf" | "card";

/** How a file of type `mime` is shown. WebKitGTK, the desktop app's webview on Linux, cannot draw a PDF. */
export function previewOf(mime: string, desktopLinux: boolean, pdfIframe = PDF_IFRAME_PREVIEW): Preview {
  if (IMAGES.has(mime)) return "image";
  if (mime === "text/plain") return "text";
  if (mime === "application/pdf" && pdfIframe && !desktopLinux) return "pdf";
  return "card";
}

const onDesktopLinux = () => isDesktop() && /linux/i.test(navigator.userAgent);

function PickFile({ id, title, on, onPick }: { id: string; title: string; on: boolean; onPick: () => void }) {
  const t = useT();
  const info = useFileInfo(id);
  const file = info.data;
  return (
    <button type="button" className={s.pick} aria-pressed={on} onClick={onPick}>
      <span aria-hidden="true" className={s.icon}><Icon name={file?.mime === "text/plain" ? "text" : "file"} size={18} strokeWidth={2} /></span>
      <span className={s.main}>
        <span className={s.name}>{file ? file.name || title : info.error ? t("files.uploadFailed") : t("common.loading")}</span>
        {file && <span className={s.meta}>{sizeLabel(file.size)}</span>}
      </span>
    </button>
  );
}

function TextPreview({ file, name }: { file: FileRef; name: string }) {
  const t = useT();
  // The start of the file, and whether the read ran into the cap. The size alone cannot say: the desktop protocol gives none.
  const read = useQuery<{ text: string; capped: boolean }>(`/files/${file.id}/text`, async () => {
    const text = await readFileText(file.id, TEXT_MAX);
    // A cut inside a character leaves up to 3 bytes of it, which the decoder turns into one U+FFFD.
    return { text, capped: new TextEncoder().encode(text).length >= TEXT_MAX - 3 };
  });
  if (read.error) return <p role="alert" className="alert alert-danger">{t("gw.textFailed")}</p>;
  if (read.data === undefined) return <p className="loading" role="status">{t("common.loading")}</p>;
  const truncated = file.size > 0 ? file.size > TEXT_MAX : read.data.capped;
  const text = truncated && read.data.text.endsWith("\ufffd") ? read.data.text.slice(0, -1) : read.data.text;
  return (
    <>
      <pre className={s.text} tabIndex={0} aria-label={name}>{text}</pre>
      {truncated && <p className={s.note}>{t("gw.truncated")}</p>}
    </>
  );
}

function Shown({ id, title, pdfIframe }: { id: string; title: string; pdfIframe: boolean }) {
  const t = useT();
  const info = useFileInfo(id);
  const file = info.data;
  if (info.error) return <p role="alert" className="alert alert-danger">{errorText(info.error)}</p>;
  if (!file) return <p className="loading" role="status">{t("common.loading")}</p>;
  const name = file.name || title;
  const kind = previewOf(file.mime, onDesktopLinux(), pdfIframe);
  return (
    <section aria-label={t("gw.preview", { name })} className={s.preview}>
      {kind === "image" && (
        // eslint-disable-next-line @next/next/no-img-element -- the API serves the file with its own policy; there is nothing to optimize
        <img className={s.img} src={fileUrl(id)} alt={name} />
      )}
      {kind === "pdf" && <iframe className={s.frame} src={fileUrl(id)} title={name} referrerPolicy="no-referrer" />}
      {kind === "text" && <TextPreview file={file} name={name} />}
      {kind === "card" && (
        <FileCard file={file} title={title}>
          <button type="button" className="btn btn-primary" onClick={() => void openFile(file).catch((e: unknown) => notice(e instanceof Error ? e.message : String(e)))}>
            {file.mime === "application/pdf" && !isDesktop() ? t("gw.openTab") : t("files.open")}
          </button>
        </FileCard>
      )}
    </section>
  );
}

/** The files of one attempt of a `file` assignment, one of them shown in place, and the student's comment under them. Mounted afresh for every attempt (`key`), so each starts at its first file. */
export function FilePreview({ content, title, pdfIframe = pdfPreview.iframe }: { content: unknown; title: string; pdfIframe?: boolean }) {
  const t = useT();
  const { files = [], comment } = fileContentOf(content);
  const [picked, setPicked] = useState<string | null>(null);
  const shown = picked && files.includes(picked) ? picked : files[0];
  return (
    <div>
      {files.length === 0 ? <p className="muted">{t("fa.noFilesInAttempt")}</p> : (
        <ul className={s.list} aria-label={t("fa.attemptFiles")}>
          {files.map((id) => <li key={id}><PickFile id={id} title={title} on={id === shown} onPick={() => setPicked(id)} /></li>)}
        </ul>
      )}
      {shown && <Shown id={shown} title={title} pdfIframe={pdfIframe} />}
      {comment && !isBlankDoc(comment) && (
        <div className={s.comment}>
          <p className={s.commentLabel}>{t("gw.studentComment")}</p>
          <BlockRenderer doc={comment} size={15} />
        </div>
      )}
    </div>
  );
}
