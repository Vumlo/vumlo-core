"use client";

import { useT } from "@/lib/i18n";
import { fullName, useMe } from "@/lib/session";
import s from "@/components/console/ConsoleShell.module.css";

export default function ConsoleHome() {
  const t = useT();
  const me = useMe();
  return (
    <div className={s.home}>
      <h1>{t("console.title")}</h1>
      <p>{me.data ? t("console.signedInAs", { name: fullName(me.data) }) : null}</p>
    </div>
  );
}
