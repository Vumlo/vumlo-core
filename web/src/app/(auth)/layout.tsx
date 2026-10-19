"use client";

import type { ReactNode } from "react";
import { BrandMark } from "@/components/BrandMark";
import { Icon } from "@/components/Icon";
import { useInstitution } from "@/lib/api/hooks/institution";
import { LANGUAGES, useLanguage, useT, type Language } from "@/lib/i18n";
import { useTheme } from "@/lib/theme";
import s from "./auth.module.css";

/** The frame shared by sign in, registration, password reset and the other pages you reach without a session. */
export default function AuthLayout({ children }: { children: ReactNode }) {
  const t = useT();
  const institution = useInstitution();
  const { theme, toggle } = useTheme();
  const { language, setLanguage } = useLanguage();
  const name = institution.data?.name ?? "Vumlo";
  return (
    <div className={s.page}>
      <div className={s.center}>
        <div className={s.col}>
          <div className={s.brand}>
            <BrandMark name={name} tile={s.logo} img={s.logoImg} />
            <div>
              <div className={s.brandName}>{name}</div>
              <div className={s.brandSub}>{t("auth.tagline")}</div>
            </div>
          </div>
          {children}
        </div>
      </div>
      <footer className={s.footer}>
        <label className={s.langLabel}>
          <Icon name="globe" size={18} />
          <span className="sr-only">{t("language.label")}</span>
          <select value={language} onChange={(e) => setLanguage(e.target.value as Language)}>
            {LANGUAGES.map((l) => <option key={l.id} value={l.id}>{l.name}</option>)}
          </select>
        </label>
        <button type="button" className={s.themeBtn} onClick={toggle}>
          <Icon name={theme === "dark" ? "sun" : "moon"} size={18} />
          {theme === "dark" ? t("theme.light") : t("theme.dark")}
        </button>
      </footer>
    </div>
  );
}
