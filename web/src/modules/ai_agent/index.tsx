"use client";

import { Panel } from "./Panel";
import s from "./Buddy.module.css";

/** The module's own page, `/m?name=ai_agent`: the same chat, full size. */
export default function BuddyPage() {
  return (
    <div className={s.page}>
      <Panel layout="page" />
    </div>
  );
}
