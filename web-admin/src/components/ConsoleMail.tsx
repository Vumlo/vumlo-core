import { useState } from "react";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { StatusPill } from "@/components/StatusPill";
import { errorText } from "@/lib/api/client";
import { useT } from "@/lib/i18n";
import { useMailMessages, useMailStatus, useRetryMail } from "@console/hooks/mail";
import s from "./Admin.module.css";

const STATES = ["pending", "sent", "dead", "cancelled"] as const;
const STATE_KEY = { pending: "ml.pending", sent: "ml.sent", dead: "ml.dead", cancelled: "ml.cancelled" } as const;

/** The mail queue and journal (ADR-0087, ADR-0095): counts, the newest messages, and putting dead mail back. */
export function ConsoleMail() {
  const t = useT();
  const status = useMailStatus();
  const [state, setState] = useState("");
  const [plugin, setPlugin] = useState("");
  const journal = useMailMessages({ state, plugin });
  const retry = useRetryMail();
  const [asking, setAsking] = useState(false);
  const [result, setResult] = useState<{ requeued: number; erased: number } | null>(null);
  const st = status.data;

  async function retryAll() {
    const answer = await retry.run();
    if (answer) {
      setResult(answer);
      setAsking(false);
    }
  }
  return (
    <>
      <div className={s.head}><div><h1>{t("ml.title")}</h1><p className="muted" style={{ marginTop: 4 }}>{t("ml.lead")}</p></div></div>
      {!st ? (
        <p className="loading" role="status">{t("common.loading")}</p>
      ) : (
        <div className="stack" style={{ gap: 20 }}>
          {!st.configured && <p role="alert" className="alert alert-info">{t("ml.notConfigured")}</p>}
          <section aria-label={t("ml.last24")} className={`card ${s.section}`}>
            <div className="row" style={{ justifyContent: "space-between", flexWrap: "wrap", gap: 8 }}>
              <h2 style={{ fontSize: 18, fontWeight: 600 }}>{t("ml.last24")}</h2>
              {st.transport && <span className="muted" style={{ fontSize: 14 }}>{t("ml.transport", { transport: st.transport })}</span>}
            </div>
            <dl className={s.stats}>
              <div><dt>{t("ml.pending")}</dt><dd>{st.pending}</dd></div>
              <div><dt>{t("ml.sent")}</dt><dd>{st.sent_24h}</dd></div>
              <div><dt>{t("ml.dead")}</dt><dd>{st.dead_now}</dd></div>
              <div><dt>{t("ml.cancelled")}</dt><dd>{st.cancelled_24h}</dd></div>
            </dl>
            {st.oldest_pending && <p className="muted" style={{ fontSize: 14 }}>{t("ml.oldest", { at: new Date(st.oldest_pending).toLocaleString() })}</p>}
            {st.dead_by.length > 0 && (
              <>
                <p style={{ fontWeight: 500, margin: "12px 0 4px" }}>{t("ml.deadBy")}</p>
                <ul className={s.deadBy}>
                  {st.dead_by.map((d) => <li key={`${d.plugin_id}/${d.kind}`}>{d.plugin_id ?? t("ml.core")} · {d.kind}: {d.dead}</li>)}
                </ul>
              </>
            )}
            <div className="row" style={{ marginTop: 16, gap: 12, alignItems: "center", flexWrap: "wrap" }}>
              <button type="button" className="btn" disabled={st.dead_now === 0} onClick={() => { setResult(null); setAsking(true); }}>{t("ml.retry")}</button>
              {result && (
                <p role="status" className="muted" style={{ margin: 0 }}>
                  {t("ml.retried", { n: result.requeued })}{result.erased > 0 && <> {t("ml.erased", { n: result.erased })}</>}
                </p>
              )}
            </div>
          </section>
          <section aria-labelledby="ml-j" className={`card ${s.section}`}>
            <h2 id="ml-j" style={{ fontSize: 18, fontWeight: 600, marginBottom: 12 }}>{t("ml.journal")}</h2>
            <div className="row" style={{ gap: 12, flexWrap: "wrap", marginBottom: 12 }}>
              <div className="field">
                <label htmlFor="ml-state">{t("ml.state")}</label>
                <select id="ml-state" className="input" value={state} onChange={(e) => setState(e.target.value)}>
                  <option value="">{t("ml.allStates")}</option>
                  {STATES.map((x) => <option key={x} value={x}>{t(STATE_KEY[x])}</option>)}
                </select>
              </div>
              <div className="field">
                <label htmlFor="ml-plugin">{t("ml.plugin")}</label>
                <input id="ml-plugin" className="input" value={plugin} onChange={(e) => setPlugin(e.target.value)} />
              </div>
            </div>
            {!journal.data ? (
              <p className="loading" role="status">{t("common.loading")}</p>
            ) : journal.data.items.length === 0 ? (
              <p className="muted">{t("ml.none")}</p>
            ) : (
              <div className="table-wrap">
                <table className="table" aria-label={t("ml.journal")}>
                  <thead>
                    <tr>
                      {(["time", "state", "plugin", "kind", "subject", "attempts", "person", "error"] as const).map((c) => <th key={c} scope="col">{t(`ml.col.${c}`)}</th>)}
                    </tr>
                  </thead>
                  <tbody>
                    {journal.data.items.map((m) => (
                      <tr key={m.id}>
                        <td>{new Date(m.created_at).toLocaleString()}</td>
                        <td><StatusPill tone={m.state === "sent" ? "ok" : m.state === "dead" ? "danger" : "plain"}>{t(STATE_KEY[m.state as (typeof STATES)[number]] ?? "ml.pending")}</StatusPill></td>
                        <td>{m.plugin_id ?? t("ml.core")}</td>
                        <td>{m.kind}</td>
                        <td>{m.subject}</td>
                        <td>{m.attempts}</td>
                        <td title={m.user_id}><code>{m.user_id.slice(0, 8)}</code></td>
                        <td>{m.last_error}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </section>
        </div>
      )}
      {asking && st && (
        <ConfirmDialog title={t("ml.retryTitle")} confirm={t("ml.retryConfirm")} pending={retry.pending} error={errorText(retry.error)} onClose={() => { setAsking(false); retry.reset(); }} onConfirm={() => void retryAll()}>
          {t("ml.retryBody", { n: st.dead_now })}
        </ConfirmDialog>
      )}
    </>
  );
}
