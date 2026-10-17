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
import s from "./Author.module.css";

const TYPES: [FormFieldType, string, IconName][] = [["short", "Short text", "text"], ["long", "Long text", "text"], ["choice", "Choose from a list", "form"], ["people", "Classmates", "user"], ["link", "Link", "link"], ["number", "Number", "quiz"], ["date", "Date", "calendar"]];
const typeName = (t: FormFieldType) => TYPES.find(([k]) => k === t)![1];

type Rule = { key: string; label: string; kind: "switch" | "input"; desc?: string };
const RULES: Record<FormFieldType, Rule[]> = {
  short: [{ key: "max_length", label: "Longest answer", kind: "input", desc: "Characters" }, { key: "unique", label: "Each answer once", kind: "switch", desc: "Two students can't give the same answer." }],
  long: [{ key: "max_length", label: "Longest answer", kind: "input", desc: "Characters" }],
  choice: [{ key: "one_team_per_option", label: "One answer per option", kind: "switch", desc: "A taken option disappears for everyone else." }, { key: "multiple", label: "Let students pick several", kind: "switch" }],
  people: [{ key: "team_size", label: "Team size", kind: "input", desc: "Including the student who fills it in" }, { key: "one_team_per_student", label: "One team per student", kind: "switch", desc: "Someone already in a team can't be added again." }],
  link: [{ key: "allowed_sites", label: "Allowed sites, optional", kind: "input", desc: "Leave empty to accept any https link" }, { key: "https_only", label: "Must start with https://", kind: "switch" }],
  number: [],
  date: [],
};

function summary(f: FormField): string {
  switch (f.type) {
    case "short": case "long": return `Up to ${f.rules.max_length ?? "any number of"} characters${f.rules.unique ? ", each answer once" : ""}`;
    case "choice": return `${plural(f.options.length, "option")}${f.rules.one_team_per_option ? ", each one to a single answer" : ""}`;
    case "people": return `${f.rules.team_size ?? "Any number of"} classmates, the student filling it in is added on their own`;
    case "link": return f.rules.https_only ? "Must start with https://" : "Any link";
    default: return typeName(f.type);
  }
}

function csv(rows: string[][]) {
  return rows.map((r) => r.map((c) => `"${c.replace(/"/g, '""')}"`).join(",")).join("\n");
}

export function FormBuilder({ item, courseId }: { item: Item; courseId: string }) {
  const update = useUpdateItem(courseId);
  const publish = usePublishItem(courseId);
  const responses = useFormResponses(item.id);
  const [title, setTitle] = useState(item.title);
  const [cfg, setCfg] = useState<FormConfig>(() => (item.assignment!.config as unknown as FormConfig).fields ? (item.assignment!.config as unknown as FormConfig) : { note: "", fields: [] });
  const [sel, setSel] = useState(0);
  const [view, setView] = useState<"fields" | "responses">("fields");
  const [paste, setPaste] = useState<string | null>(null);
  const [reminded, setReminded] = useState<string | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  const draft = useMemo(() => ({ title, cfg }), [title, cfg]);
  const auto = useAutosave(draft, (d) => update.run(item.id, { title: d.title.trim() || item.title, config: d.cfg as unknown as Record<string, unknown> }));
  const a = item.assignment!;
  const isDraft = item.status === "draft";
  const f = cfg.fields[Math.min(sel, cfg.fields.length - 1)];
  const fi = f ? cfg.fields.indexOf(f) : -1;
  const setF = (i: number, fn: (x: FormField) => FormField) => setCfg((c) => ({ ...c, fields: c.fields.map((x, k) => (k === i ? fn(x) : x)) }));
  const add = (t: FormFieldType) => {
    const opts = t === "choice" ? [{ id: newId(), text: "", taken_by: null }, { id: newId(), text: "", taken_by: null }] : [];
    setCfg((c) => ({ ...c, fields: [...c.fields, { id: newId(), type: t, label: "", help: "", required: false, options: opts, rules: {}, has_answers: false }] }));
    setSel(cfg.fields.length);
  };
  const r = responses.data;
  const teams = r?.rows.length ?? 0;

  function download() {
    const header = ["Team", "Variant", "Members", "Repository", "Submitted"];
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
      <AuthorBar courseId={courseId} itemId={item.id} title={title} chip={isDraft ? { text: "Draft, students can't see it", tone: "draft" } : { text: "Published", tone: "ok" }} saved={auto.savedAt} pending={auto.pending} failed={auto.failed}>
        <Link href={`/courses/${courseId}/items/${item.id}`} className="btn">Preview as student</Link>
        {isDraft && <button type="button" className="btn btn-primary" disabled={publish.pending} onClick={async () => { await auto.flush(); await publish.run(item.id); }}>Publish form</button>}
      </AuthorBar>
      <p className="muted" style={{ fontSize: 14, marginBottom: 8 }}><strong style={{ color: "var(--asg-fg)" }}>Assignment</strong> Form{a.due_at ? `, due ${dueLabel(a.due_at)}` : ""}, not graded</p>
      <label htmlFor="ftitle" className="sr-only">Form title</label>
      <input id="ftitle" className={s.titleInput} value={title} onChange={(e) => setTitle(e.target.value)} />
      <div role="tablist" aria-label="Form views" className={s.tabsRow}>
        <button type="button" role="tab" aria-selected={view === "fields"} onClick={() => setView("fields")}>Fields</button>
        <button type="button" role="tab" aria-selected={view === "responses"} onClick={() => setView("responses")}>Responses{teams > 0 && <span className={s.count}>{teams}</span>}</button>
      </div>

      {view === "fields" ? (
        <div className={s.shell}>
          <nav aria-label="Fields" className={`card ${s.qnav}`}>
            <h2 style={{ fontSize: 15, fontWeight: 600 }}>Fields</h2>
            <ol>
              {cfg.fields.map((x, i) => (
                <li key={x.id}><button type="button" className={s.qnavBtn} aria-current={i === fi ? "step" : undefined} onClick={() => setSel(i)}><span className={s.qshort}>{x.label || "New field"}</span>{x.required && <span aria-label="required" style={{ color: "var(--danger)" }}>*</span>}</button></li>
              ))}
            </ol>
            {r && <p className="muted" style={{ fontSize: 13 }}>{teams > 0 ? `${r.total_students - r.missing.length} of ${r.total_students} students are in a team. ` : ""}You can still add fields and change their wording. A field with answers can&apos;t be deleted.</p>}
          </nav>
          <div className={s.canvas}>
            <div className="card card-pad" style={{ marginBottom: 14 }}>
              <label htmlFor="note" style={{ fontSize: 13, fontWeight: 600, color: "var(--muted)" }}>Note at the top of the form</label>
              <textarea id="note" className="textarea" style={{ minHeight: 70, marginTop: 6 }} value={cfg.note} onChange={(e) => setCfg((c) => ({ ...c, note: e.target.value }))} />
            </div>
            <div className="stack" style={{ gap: 14 }}>
              {cfg.fields.map((x, i) => {
                const selected = i === fi;
                return (
                  <article key={x.id} aria-label={x.label || "New field"} className={s.card} data-selected={selected} onClick={() => { if (!selected) setSel(i); }}>
                    <div className={s.cardHead}><Icon name={TYPES.find(([k]) => k === x.type)![2]} size={16} /><strong>{typeName(x.type)}</strong><span style={{ marginLeft: "auto", color: x.required ? "var(--text)" : undefined }}>{x.required ? "Required" : "Optional"}</span></div>
                    {!selected ? (
                      <><p style={{ fontSize: 15, fontWeight: 500 }}>{x.label || <span className="muted">New field</span>}</p><p className="muted" style={{ fontSize: 13 }}>{summary(x)}</p></>
                    ) : (
                      <div className="stack" style={{ gap: 12 }}>
                        <div className="field"><label htmlFor={`l-${x.id}`}>Label students see</label><input id={`l-${x.id}`} className="input" value={x.label} onChange={(e) => setF(i, (y) => ({ ...y, label: e.target.value }))} /></div>
                        <div className="field"><label htmlFor={`h-${x.id}`}>Hint under the field, optional</label><input id={`h-${x.id}`} className="input" value={x.help} onChange={(e) => setF(i, (y) => ({ ...y, help: e.target.value }))} /></div>
                        {x.type === "choice" && (
                          <div>
                            <div className="row" style={{ justifyContent: "space-between", fontSize: 13, fontWeight: 600, color: "var(--muted)", marginBottom: 6 }}><span>Options</span><span>Taken by</span></div>
                            <div role="group" aria-label="Options" className="stack" style={{ gap: 8 }}>
                              {x.options.map((o, oi) => (
                                <div key={o.id} className={s.optRow} style={{ background: o.taken_by ? "var(--sunken)" : undefined }}>
                                  <input type="text" aria-label={`Option ${oi + 1}`} value={o.text} placeholder={`Option ${oi + 1}`} onChange={(e) => setF(i, (y) => ({ ...y, options: y.options.map((z) => (z.id === o.id ? { ...z, text: e.target.value } : z)) }))} />
                                  <span style={{ fontSize: 13, color: o.taken_by ? "var(--text)" : "var(--muted)" }}>{o.taken_by ?? "Free"}</span>
                                  <button type="button" className="icon-btn" style={{ width: 32, height: 32 }} aria-label={`Remove ${o.text || `option ${oi + 1}`}`} disabled={!!o.taken_by} title={o.taken_by ? "A team already chose this option" : undefined} onClick={() => setF(i, (y) => ({ ...y, options: y.options.filter((z) => z.id !== o.id) }))}><Icon name="close" size={15} strokeWidth={2} /></button>
                                </div>
                              ))}
                            </div>
                            <div className="row" style={{ gap: 8, marginTop: 8 }}>
                              <button type="button" className="btn" style={{ minHeight: 36 }} onClick={() => setF(i, (y) => ({ ...y, options: [...y.options, { id: newId(), text: "", taken_by: null }] }))}><Icon name="plus" size={15} strokeWidth={2.2} />Add option</button>
                              <button type="button" className="btn" style={{ minHeight: 36 }} onClick={() => setPaste("")}>Paste a list</button>
                            </div>
                          </div>
                        )}
                      </div>
                    )}
                  </article>
                );
              })}
              {cfg.fields.length === 0 && <p className="empty">This form has no fields yet. Add one below.</p>}
            </div>
            <div className="row" style={{ gap: 8, flexWrap: "wrap", marginTop: 16 }}>
              <span style={{ fontSize: 14, fontWeight: 500 }}>Add a field</span>
              {TYPES.map(([t, l, ic]) => <button key={t} type="button" className="btn" style={{ minHeight: 36 }} onClick={() => add(t)}><Icon name={ic} size={15} />{l}</button>)}
            </div>
          </div>
          <aside aria-labelledby="fs-h" className={`card ${s.panel}`}>
            {f ? (
              <div className={s.panelBody}>
                <h2 id="fs-h" style={{ fontSize: 16, fontWeight: 600 }}>{f.label || "New field"}</h2>
                <div className="field">
                  <label htmlFor="ftype">Field type</label>
                  <select id="ftype" className="select" disabled={f.has_answers} value={f.type} onChange={(e) => setF(fi, (y) => ({ ...y, type: e.target.value as FormFieldType, rules: {}, options: e.target.value === "choice" && y.options.length === 0 ? [{ id: newId(), text: "", taken_by: null }, { id: newId(), text: "", taken_by: null }] : y.options }))}>
                    {TYPES.map(([k, l]) => <option key={k} value={k}>{l}</option>)}
                  </select>
                  {f.has_answers && <p className="hint">Locked, students already answered.</p>}
                </div>
                <div className={s.switchRow}><span style={{ fontSize: 14 }}>Required</span><Switch checked={f.required} onChange={(v) => setF(fi, (y) => ({ ...y, required: v }))} label="Required" /></div>
                {RULES[f.type].map((rule) => (
                  <div key={rule.key}>
                    {rule.kind === "switch" ? (
                      <div className={s.switchRow}><span><span style={{ display: "block", fontSize: 14 }}>{rule.label}</span>{rule.desc && <span className="muted" style={{ fontSize: 13 }}>{rule.desc}</span>}</span><Switch checked={!!f.rules[rule.key]} onChange={(v) => setF(fi, (y) => ({ ...y, rules: { ...y.rules, [rule.key]: v } }))} label={rule.label} /></div>
                    ) : (
                      <div className="field"><label htmlFor={`r-${rule.key}`}>{rule.label}</label><input id={`r-${rule.key}`} className="input" value={String(f.rules[rule.key] ?? "")} onChange={(e) => setF(fi, (y) => ({ ...y, rules: { ...y.rules, [rule.key]: e.target.value } }))} />{rule.desc && <p className="hint">{rule.desc}</p>}</div>
                    )}
                  </div>
                ))}
                <div className={s.row2}>
                  <button type="button" className={s.blockBtn} onClick={() => setCfg((c) => { const copy = { ...JSON.parse(JSON.stringify(f)), id: newId(), has_answers: false } as FormField; const list = c.fields.slice(); list.splice(fi + 1, 0, copy); return { ...c, fields: list }; })}><Icon name="copy" size={15} strokeWidth={2} />Duplicate field</button>
                  <button type="button" className={s.blockBtn} disabled={f.has_answers} title={f.has_answers ? "Students already answered this field" : undefined} onClick={() => { setCfg((c) => ({ ...c, fields: c.fields.filter((x) => x.id !== f.id) })); setSel(Math.max(0, fi - 1)); }}><Icon name="trash" size={15} strokeWidth={2} />Delete field</button>
                </div>
                <button type="button" className="btn btn-quiet" style={{ justifyContent: "flex-start", padding: 0 }} aria-expanded={showSettings} onClick={() => setShowSettings((v) => !v)}>Deadline and grading are in the assignment settings</button>
                {showSettings && <AssignmentSettings item={item} courseId={courseId} pointsLocked />}
              </div>
            ) : <div className={s.panelBody}><p id="fs-h" className="muted" style={{ fontSize: 14 }}>Add a field to set its type and rules.</p></div>}
          </aside>
        </div>
      ) : (
        <>
          <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 14 }}>
            <p style={{ flex: "1 1 280px", fontSize: 14 }}>
              {r ? `${plural(teams, "team")}, ${r.total_students - r.missing.length} of ${r.total_students} students.${r.missing.length ? ` ${r.missing.join(", ")} ${r.missing.length === 1 ? "isn't" : "aren't"} in a team yet.` : ""}` : "Loading…"}
            </p>
            <button type="button" className="btn" disabled={!r || r.missing.length === 0} onClick={() => setReminded(`Reminder sent to ${plural(r!.missing.length, "student")}.`)}>Remind who hasn&apos;t submitted</button>
            <button type="button" className="btn" disabled={!r || teams === 0} onClick={download}><Icon name="download" size={16} strokeWidth={2} />Download CSV</button>
          </div>
          {reminded && <p role="status" className="alert alert-ok" style={{ marginBottom: 12 }}>{reminded}</p>}
          <div className={`card ${s.responses}`}>
            <table className="table">
              <thead><tr><th scope="col">Team</th><th scope="col">Variant</th><th scope="col">Members</th><th scope="col">Repository</th><th scope="col">Submitted</th></tr></thead>
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
            {r && teams === 0 && <p className="empty">Nobody has submitted yet.</p>}
          </div>
        </>
      )}
      {paste !== null && f && (
        <Dialog title="Paste a list" description="One option per line." onClose={() => setPaste(null)}
          actions={<><button type="button" className="btn" onClick={() => setPaste(null)}>Cancel</button><button type="button" className="btn btn-primary" onClick={() => { const lines = paste.split("\n").map((l) => l.trim()).filter(Boolean); setF(fi, (y) => ({ ...y, options: [...y.options.filter((o) => o.text || o.taken_by), ...lines.map((text) => ({ id: newId(), text, taken_by: null }))] })); setPaste(null); }}>Add options</button></>}>
          <label htmlFor="paste" className="sr-only">Options, one per line</label>
          <textarea id="paste" className="textarea" style={{ minHeight: 160 }} value={paste} onChange={(e) => setPaste(e.target.value)} />
        </Dialog>
      )}
    </div>
  );
}
