
import { useState } from "react";
import { errorText } from "@/lib/api/client";
import { useSetPluginSetting, useUnsetPluginSetting } from "@console/hooks/plugins-admin";
import type { AdminPluginSetting } from "@/lib/api/types";
import { useT } from "@/lib/i18n";
import s from "./Admin.module.css";

/** What the server takes: a string as it is, a number, a boolean or a list as JSON text. */
function wire(setting: AdminPluginSetting, text: string, checked: boolean): string {
  if (setting.kind === "boolean") return checked ? "true" : "false";
  if (setting.kind === "array") {
    const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
    return JSON.stringify(setting.item_kind === "number" ? lines.map(Number) : lines);
  }
  return text;
}

/** The value in force as the form shows it: a list is one entry to a line, and a secret is never shown. */
function shown(setting: AdminPluginSetting): string {
  if (setting.secret || setting.value === null || setting.value === undefined) return "";
  if (Array.isArray(setting.value)) return setting.value.join("\n");
  return String(setting.value);
}

/**
 * One institution setting of a plugin, built from the spec its manifest declared: the type picks the input, the
 * description is its help, and a secret is write-only (ADR-0072, ADR-0095).
 */
export function SettingField({ plugin, setting }: { plugin: string; setting: AdminPluginSetting }) {
  const t = useT();
  const save = useSetPluginSetting(plugin, setting.key);
  const unset = useUnsetPluginSetting(plugin, setting.key);
  const [text, setText] = useState(shown(setting));
  const [checked, setChecked] = useState(setting.value === true);
  const id = `setting-${setting.key}`;
  const hint = `${id}-hint`;
  const state = setting.is_set ? "pl.state.set" : setting.default !== null && setting.default !== undefined ? "pl.state.default" : "pl.state.notSet";
  const error = errorText(save.error ?? unset.error);

  async function submit() {
    if (await save.run(wire(setting, text, checked)) && setting.secret) setText("");
  }
  async function resetToDefault() {
    if (await unset.run()) {
      setText(shown({ ...setting, value: setting.default }));
      setChecked(setting.default === true);
    }
  }

  const common = { id, "aria-describedby": hint, "aria-invalid": save.error ? true : undefined } as const;
  return (
    <div className={s.setting}>
      <div className={s.settingHead}>
        <label htmlFor={id} style={{ fontWeight: 600 }}>{setting.key}</label>
        <span className="pill" data-testid={`state-${setting.key}`}>{t(state)}</span>
      </div>
      <p id={hint} className="muted" style={{ fontSize: 14, margin: "2px 0 8px" }}>
        {setting.description}
        {setting.secret && <> {t("pl.secretHint")}</>}
        {setting.kind === "array" && <> {t("pl.listHint")}</>}
        {setting.default !== null && setting.default !== undefined && !setting.secret && <> {t("pl.default", { value: Array.isArray(setting.default) ? setting.default.join(", ") : String(setting.default) })}</>}
      </p>
      <div className={s.settingRow}>
        {setting.kind === "boolean" ? (
          <input {...common} type="checkbox" checked={checked} onChange={(e) => setChecked(e.target.checked)} />
        ) : setting.kind === "array" ? (
          <textarea {...common} className="input" rows={3} value={text} onChange={(e) => setText(e.target.value)} />
        ) : setting.options ? (
          <select {...common} className="input" value={text} onChange={(e) => setText(e.target.value)}>
            {!setting.is_set && <option value="" />}
            {setting.options.map((o) => <option key={o} value={o}>{o}</option>)}
          </select>
        ) : setting.kind === "number" ? (
          <input {...common} className="input" type="number" min={setting.minimum ?? undefined} max={setting.maximum ?? undefined} value={text} onChange={(e) => setText(e.target.value)} />
        ) : (
          <input {...common} className="input" type={setting.secret ? "password" : "text"} autoComplete="off" value={text} onChange={(e) => setText(e.target.value)} />
        )}
        <button type="button" className="btn btn-primary" aria-label={t("pl.save", { key: setting.key })} disabled={save.pending || (setting.kind !== "boolean" && text === "")} onClick={() => void submit()}>
          {t("cm.save")}
        </button>
        {setting.is_set && (
          <button type="button" className="btn" aria-label={t("pl.useDefault", { key: setting.key })} disabled={unset.pending} onClick={() => void resetToDefault()}>
            {t("pl.state.default")}
          </button>
        )}
      </div>
      {error && <p role="alert" className="field-error" style={{ marginTop: 6 }}>{error}</p>}
    </div>
  );
}
