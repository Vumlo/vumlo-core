"use client";

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { BlockEditor } from "@/components/doc/BlockEditor";
import { BlockRenderer } from "@/components/doc/BlockRenderer";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { Icon } from "@/components/Icon";
import { ApiError, errorText } from "@/lib/api/client";
import { useEditorProfiles } from "@/lib/api/hooks/content";
import { useCourseInfo } from "@/lib/api/hooks/courses";
import { useMySubmission, useSaveDraft, useSubmitDraft } from "@/lib/api/hooks/assignments";
import { useFileInfo } from "@/lib/api/hooks/files";
import { useMyGrades } from "@/lib/api/hooks/work";
import { deadline } from "@/lib/assignment";
import { ownerName, typeLabel } from "@/lib/course";
import { useAutosave } from "@/lib/autosave";
import { acceptsName, fileConfigOf, fileContentOf, fileContentToWire } from "@/lib/fileAssignment";
import { openFile, sizeLabel, uploadFile } from "@/lib/files";
import { dateTime, dayLong, initials, sinceLabel, timeOf } from "@/lib/format";
import { bodyOf, emptyDocument, type Document, type Feature, type FileRef, type Item } from "@/lib/api/types";
import { attemptRule, lateRule } from "@/components/course/StudentContent";
import { useT } from "@/lib/i18n";
import { notice } from "@/lib/notice";
import { useOffline, useQueued } from "@/lib/offline";
import { useTick } from "@/lib/useTick";
import { DueBar } from "./DueBar";
import { ItemFrame, PreviewBanner } from "./ItemFrame";
import s from "./Item.module.css";
import f from "./FileAssignment.module.css";

const STUDENT_FEATURES: Feature[] = ["text_styles", "links", "lists", "code", "math"];

/** One stored file: name and size (read from the server unless `known`), with its buttons. */
function FileRow({ id, known, title, children }: { id: string; known?: FileRef; title: string; children?: (name: string) => ReactNode }) {
  const t = useT();
  const info = useFileInfo(known ? null : id);
  const file = known ?? info.data;
  const name = file ? file.name || title : "";
  return (
    <li className={f.row}>
      <span aria-hidden="true" className={f.rowIcon}><Icon name="file" size={18} strokeWidth={2} /></span>
      <span className={f.rowMain}>
        <span className={f.rowName}>{file ? name : info.error ? t("files.uploadFailed") : t("common.loading")}</span>
        {file && <span className={f.rowMeta}>{sizeLabel(file.size)}</span>}
      </span>
      {file && <span className={f.rowActions}>{children?.(name)}</span>}
    </li>
  );
}

/** A file of a submitted attempt: its name and an Open button. */
function OpenButton({ id, name, known }: { id: string; name: string; known?: FileRef }) {
  const t = useT();
  const info = useFileInfo(known ? null : id);
  const file = known ?? info.data;
  return (
    <button type="button" className="btn" aria-label={t("fa.open", { name })} disabled={!file} onClick={() => void openFile(file!).catch((e: unknown) => notice(e instanceof Error ? e.message : String(e)))}>
      {t("files.open")}
    </button>
  );
}

/** The files and the comment of a handed-in (or locked) attempt. */
function AttemptContent({ content, title, size = 16 }: { content: unknown; title: string; size?: number }) {
  const t = useT();
  const { files = [], comment } = fileContentOf(content);
  return (
    <>
      {files.length === 0 ? <p className="muted">{t("fa.noFilesInAttempt")}</p> : (
        <ul className={f.list} style={{ marginTop: 0 }} aria-label={t("fa.attemptFiles")}>
          {files.map((id) => <FileRow key={id} id={id} title={title}>{(name) => <OpenButton id={id} name={name} />}</FileRow>)}
        </ul>
      )}
      {comment && comment.blocks.length > 0 && <div style={{ marginTop: 16 }}><BlockRenderer doc={comment} size={size} /></div>}
    </>
  );
}

type Upload = { key: number; name: string; size: number; error?: string };

/** The editor of a draft: drop zone, file list and comment. Mounted afresh for every attempt, so no file carries over. */
function FileDraft({ item, draft, disabled, last, locked }: { item: Item; draft: { content: unknown; updated_at: string } | null; disabled: boolean; last: boolean; locked: boolean }) {
  const t = useT();
  const a = item.assignment!;
  const { max_files: max, accept } = useMemo(() => fileConfigOf(a.config), [a.config]);
  const offline = useOffline();
  const profiles = useEditorProfiles();
  const save = useSaveDraft(item.id);
  const submit = useSubmitDraft(item.id);
  const features = profiles.data?.profiles.find((p) => p.name === "student")?.features ?? STUDENT_FEATURES;
  const input = useRef<HTMLInputElement>(null);
  const counter = useRef(0);
  const [initial] = useState(() => fileContentOf(draft?.content));
  const [ids, setIds] = useState<string[]>(initial.files ?? []);
  const [refs, setRefs] = useState<Record<string, FileRef>>({});
  const [uploads, setUploads] = useState<Upload[]>([]);
  const [notes, setNotes] = useState<string[]>([]);
  const [stale, setStale] = useState(false);
  const [comment, setComment] = useState<Document | null>(initial.comment ?? null);
  const [over, setOver] = useState(false);
  const [asking, setAsking] = useState(false);
  const sent = useRef<string[]>([]);
  const names = useRef(refs);
  names.current = refs;
  const accepted = useRef(accept);
  accepted.current = accept;

  const value = useMemo(() => ({ ids, comment }), [ids, comment]);
  const auto = useAutosave(value, (v) => {
    sent.current = v.ids;
    return save.run(fileContentToWire(v.ids, v.comment));
  });

  // The server refuses a file that is no longer stored (an upload nobody named for a day is swept): it leaves the list and the student is told to upload it again.
  useEffect(() => {
    const bad = save.error?.errors.filter((e) => /^files\[\d+\]/.test(e.field)) ?? [];
    if (bad.length === 0) return;
    const drop = new Set<string>();
    const messages: string[] = [];
    for (const e of bad) {
      const id = sent.current[Number(/\[(\d+)\]/.exec(e.field)![1])];
      if (id) drop.add(id);
      const name = id ? names.current[id]?.name : undefined;
      if (name !== undefined && !acceptsName(accepted.current, name)) messages.push(e.message);
      else setStale(true);
    }
    setNotes(messages);
    setIds((prev) => prev.filter((id) => !drop.has(id)));
  }, [save.error]);

  const off = disabled || offline || locked;
  const busy = uploads.some((u) => !u.error);
  const types = accept.join(", ");

  async function add(list: File[]) {
    if (list.length === 0 || off) return;
    const messages: string[] = [];
    const fit = list.filter((file) => acceptsName(accept, file.name) || (messages.push(t("fa.badType", { name: file.name, types })), false));
    const room = max - ids.length - uploads.filter((u) => !u.error).length;
    if (fit.length > room) {
      setNotes([...messages, room <= 0 ? t("fa.full", { max }) : t("fa.tooMany", { max, left: room })]);
      return;
    }
    setNotes(messages);
    setStale(false);
    const rows = fit.map((file) => ({ key: counter.current++, name: file.name, size: file.size }));
    setUploads((u) => [...u, ...rows]);
    for (const [i, file] of fit.entries()) {
      const key = rows[i]!.key;
      try {
        const ref = await uploadFile(file, { assignment: item.id });
        setRefs((r) => ({ ...r, [ref.id]: ref }));
        setIds((prev) => [...prev, ref.id]);
        setUploads((u) => u.filter((x) => x.key !== key));
      } catch (e) {
        setUploads((u) => u.map((x) => (x.key === key ? { ...x, error: errorText(e instanceof ApiError ? e : undefined) ?? t("files.uploadFailed") } : x)));
      }
    }
  }
  function choose(list: FileList | null) {
    const files = Array.from(list ?? []);
    if (input.current) input.current.value = "";
    void add(files);
  }

  async function send() {
    setAsking(false);
    // The server keeps the last stored draft: when the newest change did not reach it, nothing is sent.
    if (!(await auto.flush())) return;
    await submit.run();
  }

  const saveFailed = auto.failed && !stale && notes.length === 0;
  const savedText = saveFailed ? t("asg.saveFailed") : auto.pending ? t("asg.saving") : (auto.savedAt ?? draft?.updated_at) ? t("asg.savedAt", { time: timeOf(auto.savedAt ?? draft!.updated_at) }) : t("asg.notSaved");
  const problem = submit.error ?? (save.error && !save.error.errors.some((e) => e.field.startsWith("files[")) ? save.error : undefined);
  const canSubmit = ids.length > 0 && !busy && !off && !submit.pending;

  return (
    <section aria-labelledby="files-h">
      <div className="row" style={{ alignItems: "baseline", justifyContent: "space-between", gap: 12, marginBottom: 12 }}>
        <h2 id="files-h" className={s.h2} style={{ margin: 0 }}>{t("fa.files")}</h2>
        {!locked && <span role="status" className={s.saved} style={saveFailed ? { color: "var(--danger)" } : undefined}><Icon name={saveFailed ? "info" : "check"} size={14} strokeWidth={2.2} />{savedText}</span>}
      </div>

      {!locked && (
        <div
          className={`${f.zone} ${over ? f.zoneOver : ""} ${off ? f.zoneOff : ""}`}
          onDragOver={(e) => { e.preventDefault(); setOver(true); }}
          onDragLeave={() => setOver(false)}
          onDrop={(e) => { e.preventDefault(); setOver(false); void add(Array.from(e.dataTransfer.files)); }}
        >
          <span aria-hidden="true" className={f.zoneIcon}><Icon name="file" size={22} strokeWidth={2} /></span>
          <p className={f.zoneText}>{t("fa.drop")}</p>
          <button type="button" className="btn btn-outline" disabled={off} onClick={() => input.current?.click()}>{t("fa.choose")}</button>
          <input ref={input} type="file" hidden multiple accept={accept.length ? accept.join(",") : undefined} onChange={(e) => choose(e.target.files)} />
        </div>
      )}
      {offline && <p role="status" className="alert alert-info" style={{ marginTop: 12 }}>{t("fa.offline")}</p>}
      {notes.map((n) => <p key={n} role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{n}</p>)}
      {stale && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{t("files.reupload")}</p>}

      {ids.length + uploads.length > 0 ? (
        <ul className={f.list} aria-label={t("fa.files")}>
          {ids.map((id) => (
            <FileRow key={id} id={id} known={refs[id]} title={item.title}>
              {(name) => (
                <>
                  <OpenButton id={id} name={name} known={refs[id]} />
                  {!locked && <button type="button" className="btn" aria-label={t("fa.remove", { name })} disabled={off} onClick={() => setIds((prev) => prev.filter((x) => x !== id))}>{t("fe.remove")}</button>}
                </>
              )}
            </FileRow>
          ))}
          {uploads.map((u) => (
            <li key={u.key} className={f.row}>
              <span aria-hidden="true" className={f.rowIcon}><Icon name="file" size={18} strokeWidth={2} /></span>
              <span className={f.rowMain}>
                <span className={f.rowName}>{u.name}</span>
                {u.error ? <span role="alert" className={f.rowErr}>{u.error}</span> : <span role="status" className={f.rowMeta}>{t("fe.uploading")}</span>}
              </span>
              {u.error && <button type="button" className="btn" aria-label={t("fa.dismiss", { name: u.name })} onClick={() => setUploads((x) => x.filter((y) => y.key !== u.key))}>{t("common.done")}</button>}
            </li>
          ))}
        </ul>
      ) : (
        <p className="muted" style={{ marginTop: 16 }}>{t("fa.noFiles")}</p>
      )}

      <div className={f.comment}>
        <h2 className={s.h2}>{t("fa.comment")}</h2>
        {locked ? (comment && <BlockRenderer doc={comment} size={16} />) : (
          <div className={s.editor}>
            <BlockEditor value={initial.comment ?? emptyDocument()} onChange={setComment} features={features} label={t("fa.comment")} mode="float" minHeight={140} placeholder={t("fa.commentPlaceholder")} />
          </div>
        )}
      </div>

      {problem && <p role="alert" className="alert alert-danger" style={{ marginTop: 16 }}>{errorText(problem)}</p>}
      {!locked && (
        <div className={f.foot}>
          <button type="button" className="btn btn-submit" disabled={!canSubmit} onClick={() => (last ? setAsking(true) : void send())}>
            <Icon name="check" size={16} strokeWidth={2.4} />{t("fa.submit")}
          </button>
          {ids.length === 0 && <span className="muted" style={{ fontSize: 14 }}>{t("fa.needFile")}</span>}
        </div>
      )}
      {asking && (
        <ConfirmDialog title={t("sub.title")} confirm={t("fa.submit")} onClose={() => setAsking(false)} onConfirm={() => void send()}>
          {t("sub.last")}
        </ConfirmDialog>
      )}
    </section>
  );
}

type View = "not-started" | "draft" | "submitted" | "graded" | "closed";

/** The page of a `file` assignment (spec 9.3): a student drops files, adds a comment and submits. */
export function FileAssignmentPage({ item, courseId, staff }: { item: Item; courseId: string; staff: boolean }) {
  const t = useT();
  const a = item.assignment!;
  const info = useCourseInfo(courseId);
  const sub = useMySubmission(staff ? null : item.id);
  const grades = useMyGrades(staff ? null : courseId);
  const waiting = useQueued(`/assignments/${item.id}/submission/submit`);
  const resubmit = useSaveDraft(item.id);
  const { max_files: max, accept } = useMemo(() => fileConfigOf(a.config), [a.config]);

  // The deadline can pass while the page is open.
  useTick();
  const dl = deadline(a);
  const attempts = sub.data?.attempts ?? [];
  const latest = attempts[0];
  const draft = sub.data?.draft ?? null;
  const grade = grades.data?.items.find((g) => g.assignment_id === item.id);
  const attemptsLeft = a.max_attempts == null ? Infinity : a.max_attempts - attempts.length;
  const canEdit = !staff && dl.open && attemptsLeft > 0 && !waiting;
  const loading = !staff && (!sub.data || !grades.data);
  // Nothing is stored until the first change, so a student who has not handed in anything starts at the drop zone.
  const view: View = grade ? "graded" : staff ? "not-started" : draft || (!latest && dl.open) ? "draft" : latest ? "submitted" : "closed";
  const locked = view === "draft" && !dl.open;
  const stamp = draft?.updated_at;

  const pills = {
    "not-started": { label: t("status.notStarted"), tone: "plain" as const },
    draft: { label: stamp ? t("asg.pillSaved", { time: timeOf(stamp) }) : t("common.draft"), tone: "accent" as const },
    submitted: { label: a.max_attempts ? t("asg.submittedOf", { n: latest?.attempt ?? 1, max: a.max_attempts }) : t("asg.submittedN", { n: latest?.attempt ?? 1 }), tone: "ok" as const },
    graded: { label: t("status.graded"), tone: "ok" as const },
    closed: { label: t("status.notSubmitted"), tone: "danger" as const },
  };
  const pill = loading ? { label: t("common.loading"), tone: "plain" as const } : pills[view];

  return (
    <ItemFrame courseId={courseId} itemId={item.id}>
      {({ crumbs }) => (
        <>
          {staff && <PreviewBanner courseId={courseId} itemId={item.id} />}
          {crumbs}
          <div className={s.kind}>
            <span aria-hidden="true" className={s.kindIcon} data-kind="assignment"><Icon name="assignment" size={15} strokeWidth={2} /></span>
            <span className={s.kindLabel} data-kind="assignment">{t("cm.assignment")}</span>
            <span className="muted" style={{ fontSize: 14 }}>{typeLabel(a.type)}</span>
          </div>
          <h1 className={s.title}>{item.title}</h1>

          <DueBar due={a.due_at ?? null} open={dl.open && view !== "graded"} points={a.max_points} pill={pill} danger={!loading && (view === "closed" || locked)}>
            {view === "graded" && grade && (
              <span className={s.score}><span className={s.scoreNum}>{grade.score}</span><span className="muted" style={{ fontSize: 14 }}> / {a.max_points}</span></span>
            )}
          </DueBar>
          {waiting && <p role="status" className="alert alert-info" style={{ marginTop: 12 }}>{t("offline.queuedNote")}</p>}
          {resubmit.error && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{errorText(resubmit.error)}</p>}

          {!loading && (view === "closed" || locked) && (
            <div role="status" className={s.note}>
              <strong>{t("asg.closedTitle")}</strong> {t("asg.closedLead", { owner: ownerName(info.data) ?? t("item.yourTeacher") })}
            </div>
          )}

          {loading ? <p className="loading" role="status">{t("common.loading")}</p> : (
            <div className={s.blocks}>
              <section aria-labelledby="instr-h">
                <h2 id="instr-h" className={s.h2}>{t("item.instructions")}</h2>
                <BlockRenderer doc={bodyOf(item)} size={16} />
                <ul className={s.facts}>
                  <li>{t.plural("fa.rule.max", max)}{accept.length > 0 && ` ${t("fa.rule.accept", { types: accept.join(", ") })}`}</li>
                  {a.max_attempts != null ? <li>{t.plural("asg.canSubmit", a.max_attempts)} {t("asg.gradesLast")}</li> : <li>{attemptRule(null)} {t("asg.gradesLast")}</li>}
                  <li>{lateRule(a)}</li>
                  {a.on_deadline === "auto_submit" && <li>{t("asg.autoSubmit")}</li>}
                </ul>
              </section>

              {view === "draft" && (
                <div className={s.answer}>
                  <FileDraft key={attempts.length} item={item} draft={draft} disabled={waiting} last={attemptsLeft === 1} locked={locked} />
                  <p className="muted" style={{ margin: "12px 4px 0", fontSize: 14 }}>
                    {t("asg.savedAsYouType")} {a.max_attempts ? t("asg.usesOf", { n: attempts.length + 1, max: a.max_attempts }) : t("asg.uses", { n: attempts.length + 1 })}
                  </p>
                </div>
              )}

              {view === "submitted" && latest && (
                <section aria-labelledby="sub-h" className={s.panel}>
                  <div className={s.panelHead}>
                    <div>
                      <h2 id="sub-h" style={{ fontSize: 16, fontWeight: 600 }}>{t("grading.attempt", { n: latest.attempt })}</h2>
                      <p className="muted" style={{ fontSize: 13 }}>{t("asg.submittedWaiting", { when: sinceLabel(latest.submitted_at) })}</p>
                    </div>
                    {canEdit && <button type="button" className="btn btn-outline" onClick={() => void resubmit.run(fileContentToWire([], null))} disabled={resubmit.pending}>{t("asg.resubmit")}</button>}
                  </div>
                  <div className={s.panelBody}><AttemptContent content={latest.content} title={item.title} /></div>
                  {canEdit && (
                    <p className={s.panelFoot}>
                      {attemptsLeft === Infinity ? t("asg.again") : t.plural("asg.left", attemptsLeft)} {t("asg.resubmitNote")}
                    </p>
                  )}
                </section>
              )}

              {view === "graded" && grade && (
                <section aria-labelledby="grade-h" className={s.panel}>
                  <div className={s.gradeTop}>
                    <div className="row" style={{ flexWrap: "wrap", alignItems: "baseline", justifyContent: "space-between", gap: 8 }}>
                      <h2 id="grade-h" style={{ fontSize: 16, fontWeight: 600 }}>{t("item.grade")}</h2>
                      <span className="muted" style={{ fontSize: 13 }}>{t("item.publishedOn", { day: dayLong(grade.published_at) })}</span>
                    </div>
                    <p style={{ marginTop: 8, whiteSpace: "nowrap" }}><span className={s.gradeNum}>{grade.score}</span><span className="muted" style={{ fontSize: 17 }}> / {a.max_points}</span></p>
                  </div>
                  {grade.feedback && (
                    <div className={s.comment}>
                      <div className="row" style={{ gap: 12, marginBottom: 12 }}>
                        <span aria-hidden="true" className={s.avatar}>{initials(info.data?.owner_name ?? "", info.data?.owner_surname ?? "")}</span>
                        <div>
                          <p style={{ fontSize: 14, fontWeight: 600 }}>{ownerName(info.data)}</p>
                          <p className="muted" style={{ fontSize: 12 }}>{latest ? t("asg.commentOn", { n: latest.attempt }) : t("asg.comment")}</p>
                        </div>
                      </div>
                      <p style={{ fontSize: 16, lineHeight: 1.65 }}>{grade.feedback}</p>
                    </div>
                  )}
                  {latest && (
                    <details className={s.details}>
                      <summary>
                        <Icon name="chevronRight" size={16} strokeWidth={2} style={{ color: "var(--muted)" }} />
                        {t("fa.attemptFiles")}
                        <span className="muted" style={{ fontWeight: 400 }}>{t("asg.attemptSubmitted", { n: latest.attempt, when: dateTime(latest.submitted_at) })}</span>
                      </summary>
                      <div><AttemptContent content={latest.content} title={item.title} size={15} /></div>
                    </details>
                  )}
                </section>
              )}
            </div>
          )}
        </>
      )}
    </ItemFrame>
  );
}
