"use client";

import { useState } from "react";
import { useT } from "@/lib/i18n";
import s from "./PasswordField.module.css";

type Props = {
  id: string;
  label: string;
  value: string;
  onChange: (v: string) => void;
  autoComplete: "current-password" | "new-password";
  invalid?: boolean;
  describedBy?: string;
  trailing?: React.ReactNode;
  error?: string;
};

/** A password input with a Show button. Both fields of a change use it, so the value stays masked by default. */
export function PasswordField({ id, label, value, onChange, autoComplete, invalid, describedBy, trailing, error }: Props) {
  const t = useT();
  const [shown, setShown] = useState(false);
  return (
    <div className="field">
      <div className={s.head}>
        <label htmlFor={id}>{label}</label>
        {trailing}
      </div>
      <div className={s.wrap} data-invalid={invalid ? "true" : undefined}>
        <input
          id={id}
          type={shown ? "text" : "password"}
          autoComplete={autoComplete}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          aria-invalid={invalid ? "true" : "false"}
          aria-describedby={describedBy}
        />
        <button type="button" className={s.show} aria-label={shown ? t("password.hideLabel") : t("password.showLabel")} aria-pressed={shown} onClick={() => setShown((x) => !x)}>
          {shown ? t("password.hide") : t("password.show")}
        </button>
      </div>
      {error && <p className="hint" style={{ color: "var(--danger)" }}>{error}</p>}
    </div>
  );
}
