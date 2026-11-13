import Link from "next/link";
import { Icon } from "@/components/Icon";
import { SECTIONS } from "@console/components/sections";
import { useT } from "@/lib/i18n";
import { fullName, useMe } from "@/lib/session";
import s from "@console/components/ConsoleShell.module.css";

export function Home() {
  const t = useT();
  const me = useMe();
  return (
    <div className={s.home}>
      <h1>{t("console.title")}</h1>
      <p>{me.data ? t("console.signedInAs", { name: fullName(me.data) }) : null}</p>
      <ul className={s.sections}>
        {SECTIONS.map(([id, label, icon]) => (
          <li key={id}>
            <Link href={`/${id}`} className={s.section}>
              <Icon name={icon} size={20} />{t(label)}
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}
