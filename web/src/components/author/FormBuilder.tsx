"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { Dialog } from "@/components/Dialog";
import { Icon, type IconName } from "@/components/Icon";
import { newId } from "@/components/doc/model";
import { usePublishItem, useUpdateItem } from "@/lib/api/hooks/content";
import { useFormResponses } from "@/lib/api/hooks/assignments";
import { useAutosave } from "@/lib/autosave";
import type { FormConfig, FormField, FormFieldType } from "@/lib/api/gaps";
import type { Item } from "@/lib/api/types";
import { dueLabel, plural } from "@/lib/format";
import { AssignmentSettings } from "./AssignmentSettings";
import { AuthorBar, Switch } from "./AuthorBar";
import { translate } from "@/lib/i18n/core";
import type { MessageKey } from "@/lib/i18n/core";
import { useT } from "@/lib/i18n";
import s from "./Author.module.css";
import { itemHref } from "@/lib/routes";

const TYPES: [FormFieldType, MessageKey, IconName][] = [["short", "fb.short", "text"], ["long", "fb.long", "text"], ["choice", "fb.choice", "form"], ["people", "fb.people", "user"], ["link", "fb.link", "link"], ["number", "qb.number", "quiz"], ["date", "fb.date", "calendar"]];
const typeName = (type: FormFieldType) => translate(TYPES.find(([k]) => k === type)![1]);

type Rule = { key: string; label: MessageKey; kind: "switch" | "input"; desc?: MessageKey };
const RULES: Record<FormFieldType, Rule[]> = {
  short: [{ key: "max_length", label: "fb.maxLength", kind: "input", desc: "fb.characters" }, { key: "unique", label: "fb.unique", kind: "switch", desc: "fb.uniqueHint" }],
  long: [{ key: "max_length", label: "fb.maxLength", kind: "input", desc: "fb.characters" }],
  choice: [{ key: "one_team_per_option", label: "fb.onePerOption", kind: "switch", desc: "fb.onePerOptionHint" }, { key: "multiple", label: "fb.multiple", kind: "switch" }],
  people: [{ key: "team_size", label: "fb.teamSize", kind: "input", desc: "fb.teamSizeHint" }, { key: "one_team_per_student", label: "fb.oneTeam", kind: "switch", desc: "fb.oneTeamHint" }],
  link: [{ key: "allowed_sites", label: "fb.sites", kind: "input", desc: "fb.sitesHint" }, { key: "https_only", label: "fb.https", kind: "switch" }],
  number: [],
  date: [],
};

function summary(f: FormField): string {
  switch (f.type) {
    case "short": case "long": return `${translate("fb.upTo", { n: String(f.rules.max_length ?? translate("fb.anyNumber")) })}${f.rules.unique ? `, ${translate("fb.unique").toLowerCase()}` : ""}`;
    case "choice": return `${plural(f.options.length, "option")}${f.rules.one_team_per_option ? `, ${translate("fb.eachOne")}` : ""}`;
    case "people": return translate("fb.peopleSummary", { n: String(f.rules.team_size ?? translate("fb.anyNumber")) });
    case "link": return f.rules.https_only ? translate("fb.https") : translate("fb.anyLink");
    default: return typeName(f.type);
  }
}

function csv(rows: string[][]) {
  return rows.map((r) => r.map((c) => `"${c.replace(/"/g, '""')}"`).join(",")).join("\n");
}

export function FormBuilder({ item, courseId }: { item: Item; courseId: string }) {
  const t = useT();
  const update = useUpdateItem(courseId);
  const publish = usePublishItem(courseId);
  const responses = useFormResponses(item.id, item.assignment!.config as unknown as FormConfig);
  const [title, setTitle] = useState(item.title);
  const [cfg, setCfg] = useState<FormConfig>(() => (item.assignment!.config as unknown as FormConfig).fields ? (item.assignment!.config as unknown as FormConfig) : { note: "", fields: [] });
  const [sel, setSel] = useState(0);
  const [view, setView] = useState<"fields" | "responses">("fields");
  const [paste, setPaste] = useState<string | null>(null);
  const [reminded, setReminded] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const draft = useMemo(() => ({ title, cfg }), [title, cfg]);
  const auto = useAutosave(draft, (d) => update.run(item.id, { title: d.title.trim() || item.title, config: d.cfg as unknown as Record<string, unknown>, configType: "form" }));
  const a = item.assignment!;
  const isDraft = item.status === "draft";
  const f = cfg.fields[Math.min(sel, cfg.fields.length - 1)];
  const fi = f ? cfg.fields.indexOf(f) : -1;
  const setF = (i: number, fn: (x: FormField) => FormField) => setCfg((c) => ({ ...c, fields: c.fields.map((x, k) => (k === i ? fn(x) : x)) }));
  const add = (type: FormFieldType) => {
    const opts = type === "choice" ? [{ id: newId(), text: "", taken_by: null }, { id: newId(), text: "", taken_by: null }] : [];
    setCfg((c) => ({ ...c, fields: [...c.fields, { id: newId(), type, label: "", help: "", required: false, options: opts, rules: {}, has_answers: false }] }));
    setSel(cfg.fields.length);
  };
  const r = responses.data;
  const teams = r?.rows.length ?? 0;

  function download() {
    const header = [t("fb.team"), t("fb.variant"), t("fb.members"), t("fb.repository"), t("grading.submitted")];
    const body = (r?.rows ?? []).map((x) => [x.team, x.variant, x.members, x.repo, x.submitted_at]);
    const url = URL.createObjectURL(new Blob([csv([header, ...body])], { type: "text/csv" }));
    const link = document.createElement("a");
    link.href = url;
    link.download = `${item.title}.csv`;
    link.click();
    URL.revokeObjectURL(url);
  }

  return (
    <div className="container" style={{ paddingTop: 20 }}>
      <AuthorBar courseId={courseId} itemId={item.id} title={title} chip={isDraft ? { text: t("le.draftChip"), tone: "draft" } : { text: t("cm.published"), tone: "ok" }} saved={auto.savedAt} pending={auto.pending} failed={auto.failed}>
        <Link href={itemHref(courseId, item.id)} className="btn">{t("cm.preview")}</Link>
        {isDraft && <button type="button" className="btn btn-primary" disabled={publish.pending} onClick={async () => { await auto.flush(); await publish.run(item.id); }}>{t("fb.publish")}</button>}
      </AuthorBar>
      <p className="muted" style={{ fontSize: 14, marginBottom: 8 }}><strong style={{ color: "var(--asg-fg)" }}>{t("cm.assignment")}</strong> {t("type.form")}{a.due_at ? `, ${t("search.due", { when: dueLabel(a.due_at) })}` : ""}{t("bar.notGraded")}</p>
      <label htmlFor="ftitle" className="sr-only">{t("fb.formTitle")}</label>
      <input id="ftitle" className={s.titleInput} value={title} onChange={(e) => setTitle(e.target.value)} />
      <div role="tablist" aria-label={t("fb.views")} className={s.tabsRow}>
        <button type="button" role="tab" aria-selected={view === "fields"} onClick={() => setView("fields")}>{t("sheet.fields")}</button>
        <button type="button" role="tab" aria-selected={view === "responses"} onClick={() => setView("responses")}>{t("fb.responses")}{teams > 0 && <span className={s.count}>{teams}</span>}</button>
      </div>

      {view === "fields" ? (
        <div className={s.shell}>
          <nav aria-label={t("sheet.fields")} className={`card ${s.qnav}`}>
            <h2 style={{ fontSize: 15, fontWeight: 600 }}>{t("sheet.fields")}</h2>
            <ol>
              {cfg.fields.map((x, i) => (
                <li key={x.id}><button type="button" className={s.qnavBtn} aria-current={i === fi ? "step" : undefined} onClick={() => setSel(i)}><span className={s.qshort}>{x.label || t("fb.newField")}</span>{x.required && <span aria-label={t("sheet.required")} style={{ color: "var(--danger)" }}>*</span>}</button></li>
              ))}
            </ol>
            {r && <p className="muted" style={{ fontSize: 13 }}>{teams > 0 ? `${t("fb.inTeam", { n: r.total_students - r.missing.length, total: r.total_students })} ` : ""}{t("fb.editNote")}</p>}
          </nav>
          <div className={s.canvas}>
            <div className="card card-pad" style={{ marginBottom: 14 }}>
              <label htmlFor="note" style={{ fontSize: 13, fontWeight: 600, color: "var(--muted)" }}>{t("fb.note")}</label>
              <textarea id="note" className="textarea" style={{ minHeight: 70, marginTop: 6 }} value={cfg.note} onChange={(e) => setCfg((c) => ({ ...c, note: e.target.value }))} />
            </div>
            <div className="stack" style={{ gap: 14 }}>
              {cfg.fields.map((x, i) => {
                const selected = i === fi;
                return (
                  <article key={x.id} aria-label={x.label || t("fb.newField")} className={s.card} data-selected={selected} onClick={() => { if (!selected) setSel(i); }}>
                    <div className={s.cardHead}><Icon name={TYPES.find(([k]) => k === x.type)![2]} size={16} /><strong>{typeName(x.type)}</strong><span style={{ marginLeft: "auto", color: x.required ? "var(--text)" : undefined }}>{x.required ? t("sheet.required") : t("sheet.optional")}</span></div>
                    {!selected ? (
                      <><p style={{ fontSize: 15, fontWeight: 500 }}>{x.label || <span className="muted">{t("fb.newField")}</span>}</p><p className="muted" style={{ fontSize: 13 }}>{summary(x)}</p></>
                    ) : (
                      <div className="stack" style={{ gap: 12 }}>
                        <div className="field"><label htmlFor={`l-${x.id}`}>{t("fb.label")}</label><input id={`l-${x.id}`} className="input" value={x.label} onChange={(e) => setF(i, (y) => ({ ...y, label: e.target.value }))} /></div>
                        <div className="field"><label htmlFor={`h-${x.id}`}>{t("fb.hint")}</label><input id={`h-${x.id}`} className="input" value={x.help} onChange={(e) => setF(i, (y) => ({ ...y, help: e.target.value }))} /></div>
                        {x.type === "choice" && (
                          <div>
                            <div className="row" style={{ justifyContent: "space-between", fontSize: 13, fontWeight: 600, color: "var(--muted)", marginBottom: 6 }}><span>{t("qb.options")}</span><span>{t("fb.takenBy")}</span></div>
                            <div role="group" aria-label={t("qb.options")} className="stack" style={{ gap: 8 }}>
                              {x.options.map((o, oi) => (
                                <div key={o.id} className={s.optRow} style={{ background: o.taken_by ? "var(--sunken)" : undefined }}>
                                  <input type="text" aria-label={t("qb.option", { n: oi + 1 })} value={o.text} placeholder={t("qb.option", { n: oi + 1 })} onChange={(e) => setF(i, (y) => ({ ...y, options: y.options.map((z) => (z.id === o.id ? { ...z, text: e.target.value } : z)) }))} />
                                  <span style={{ fontSize: 13, color: o.taken_by ? "var(--text)" : "var(--muted)" }}>{o.taken_by ?? t("fb.free")}</span>
                                  <button type="button" className="icon-btn" style={{ width: 32, height: 32 }} aria-label={t("sheet.remove", { name: o.text || t("qb.option", { n: oi + 1 }) })} disabled={!!o.taken_by} title={o.taken_by ? t("fb.takenHint") : undefined} onClick={() => setF(i, (y) => ({ ...y, options: y.options.filter((z) => z.id !== o.id) }))}><Icon name="close" size={15} strokeWidth={2} /></button>
                                </div>
                              ))}
                            </div>
                            <div className="row" style={{ gap: 8, marginTop: 8 }}>
                              <button type="button" className="btn" style={{ minHeight: 36 }} onClick={() => setF(i, (y) => ({ ...y, options: [...y.options, { id: newId(), text: "", taken_by: null }] }))}><Icon name="plus" size={15} strokeWidth={2.2} />{t("qb.addOption")}</button>
                              <button type="button" className="btn" style={{ minHeight: 36 }} onClick={() => setPaste("")}>{t("fb.pasteList")}</button>
                            </div>
                          </div>
                        )}
                      </div>
                    )}
                  </article>
                );
              })}
              {cfg.fields.length === 0 && <p className="empty">{t("fb.none")}</p>}
            </div>
            <div className="row" style={{ gap: 8, flexWrap: "wrap", marginTop: 16 }}>
              <span style={{ fontSize: 14, fontWeight: 500 }}>{t("fb.addField")}</span>
              {TYPES.map(([k, , ic]) => <button key={k} type="button" className="btn" style={{ minHeight: 36 }} onClick={() => add(k)}><Icon name={ic} size={15} />{typeName(k)}</button>)}
            </div>
          </div>
          <aside aria-labelledby="fs-h" className={`card ${s.panel}`}>
            {f ? (
              <div className={s.panelBody}>
                <h2 id="fs-h" style={{ fontSize: 16, fontWeight: 600 }}>{f.label || t("fb.newField")}</h2>
                <div className="field">
                  <label htmlFor="ftype">{t("fb.fieldType")}</label>
                  <select id="ftype" className="select" disabled={f.has_answers} value={f.type} onChange={(e) => setF(fi, (y) => ({ ...y, type: e.target.value as FormFieldType, rules: {}, options: e.target.value === "choice" && y.options.length === 0 ? [{ id: newId(), text: "", taken_by: null }, { id: newId(), text: "", taken_by: null }] : y.options }))}>
                    {TYPES.map(([k]) => <option key={k} value={k}>{typeName(k)}</option>)}
                  </select>
                  {f.has_answers && <p className="hint">{t("fb.locked")}</p>}
                </div>
                <div className={s.switchRow}><span style={{ fontSize: 14 }}>{t("sheet.required")}</span><Switch checked={f.required} onChange={(v) => setF(fi, (y) => ({ ...y, required: v }))} label={t("sheet.required")} /></div>
                {RULES[f.type].map((rule) => (
                  <div key={rule.key}>
                    {rule.kind === "switch" ? (
                      <div className={s.switchRow}><span><span style={{ display: "block", fontSize: 14 }}>{t(rule.label)}</span>{rule.desc && <span className="muted" style={{ fontSize: 13 }}>{t(rule.desc)}</span>}</span><Switch checked={!!f.rules[rule.key]} onChange={(v) => setF(fi, (y) => ({ ...y, rules: { ...y.rules, [rule.key]: v } }))} label={t(rule.label)} /></div>
                    ) : (
                      <div className="field"><label htmlFor={`r-${rule.key}`}>{t(rule.label)}</label><input id={`r-${rule.key}`} className="input" value={String(f.rules[rule.key] ?? "")} onChange={(e) => setF(fi, (y) => ({ ...y, rules: { ...y.rules, [rule.key]: e.target.value } }))} />{rule.desc && <p className="hint">{t(rule.desc)}</p>}</div>
                    )}
                  </div>
                ))}
                <div className={s.row2}>
                  <button type="button" className={s.blockBtn} onClick={() => setCfg((c) => { const copy = { ...JSON.parse(JSON.stringify(f)), id: newId(), has_answers: false } as FormField; const list = c.fields.slice(); list.splice(fi + 1, 0, copy); return { ...c, fields: list }; })}><Icon name="copy" size={15} strokeWidth={2} />{t("fb.duplicate")}</button>
                  <button type="button" className={s.blockBtn} disabled={f.has_answers} title={f.has_answers ? t("fb.answeredHint") : undefined} onClick={() => { setCfg((c) => ({ ...c, fields: c.fields.filter((x) => x.id !== f.id) })); setSel(Math.max(0, fi - 1)); }}><Icon name="trash" size={15} strokeWidth={2} />{t("fb.delete")}</button>
                </div>
                <button type="button" className="btn btn-quiet" style={{ justifyContent: "flex-start", padding: 0 }} aria-expanded={showSettings} onClick={() => setShowSettings((v) => !v)}>{t("fb.deadlineIn")}</button>
                {showSettings && <AssignmentSettings item={item} courseId={courseId} pointsLocked />}
              </div>
            ) : <div className={s.panelBody}><p id="fs-h" className="muted" style={{ fontSize: 14 }}>{t("fb.addToSet")}</p></div>}
          </aside>
        </div>
      ) : (
        <>
          <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 14 }}>
            <p style={{ flex: "1 1 280px", fontSize: 14 }}>
              {r ? `${plural(teams, "team")}, ${t("fb.studentsOf", { n: r.total_students - r.missing.length, total: r.total_students })}${r.missing.length ? ` ${t.plural("fb.notInTeam", r.missing.length, { names: r.missing.join(", ") })}` : ""}` : t("common.loading")}
            </p>
            <button type="button" className="btn" disabled={!r || r.missing.length === 0} onClick={() => setReminded(t("fb.reminded", { n: plural(r!.missing.length, "student") }))}>{t("fb.remind")}</button>
            <button type="button" className="btn" disabled={!r || teams === 0} onClick={download}><Icon name="download" size={16} strokeWidth={2} />{t("fb.csv")}</button>
          </div>
          {reminded && <p role="status" className="alert alert-ok" style={{ marginBottom: 12 }}>{reminded}</p>}
          <div className={`card ${s.responses}`}>
            <table className="table">
              <thead><tr><th scope="col">{t("fb.team")}</th><th scope="col">{t("fb.variant")}</th><th scope="col">{t("fb.members")}</th><th scope="col">{t("fb.repository")}</th><th scope="col">{t("grading.submitted")}</th></tr></thead>
              <tbody>
                {(r?.rows ?? []).map((x) => (
                  <tr key={x.team}>
                    <th scope="row">{x.team}</th>
                    <td>{x.variant}</td><td>{x.members}</td><td><a href={`https://${x.repo}`} target="_blank" rel="noopener noreferrer" style={{ color: "var(--accent-fg)" }}>{x.repo}</a></td>
                    <td style={{ color: x.late ? "var(--danger)" : "var(--muted)", whiteSpace: "nowrap" }}>{dueLabel(x.submitted_at)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
            {r && teams === 0 && <p className="empty">{t("fb.nobody")}</p>}
          </div>
        </>
      )}
      {paste !== null && f && (
        <Dialog title={t("fb.pasteList")} description={t("fb.pasteLead")} onClose={() => setPaste(null)}
          actions={<><button type="button" className="btn" onClick={() => setPaste(null)}>{t("common.cancel")}</button><button type="button" className="btn btn-primary" onClick={() => { const lines = paste.split("\n").map((l) => l.trim()).filter(Boolean); setF(fi, (y) => ({ ...y, options: [...y.options.filter((o) => o.text || o.taken_by), ...lines.map((text) => ({ id: newId(), text, taken_by: null }))] })); setPaste(null); }}>{t("fb.addOptions")}</button></>}>
          <label htmlFor="paste" className="sr-only">{t("fb.pasteLabel")}</label>
          <textarea id="paste" className="textarea" style={{ minHeight: 160 }} value={paste} onChange={(e) => setPaste(e.target.value)} />
        </Dialog>
      )}
    </div>
  );
}
