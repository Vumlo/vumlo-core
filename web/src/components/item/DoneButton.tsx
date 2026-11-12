"use client";

import { Icon } from "@/components/Icon";
import { errorText } from "@/lib/api/client";
import { useMarkDone } from "@/lib/api/hooks/content";
import type { Item } from "@/lib/api/types";
import { isDone } from "@/lib/course";
import { useT } from "@/lib/i18n";

/**
 * A student's own mark that a text or file item is done (ADR-0092). It is what lets a rule wait for
 * an item that has nothing to submit. Staff and an item without a standing have no mark to make.
 */
export function DoneButton({ item, courseId }: { item: Item; courseId: string }) {
  const t = useT();
  const mark = useMarkDone(courseId);
  if (!item.standing) return null;
  const done = isDone(item);
  return (
    <div style={{ marginTop: 32 }}>
      <button type="button" className={done ? "btn btn-outline" : "btn btn-primary"} aria-pressed={done} disabled={mark.pending} onClick={() => void mark.run(item.id, !done)}>
        {done && <Icon name="check" size={16} strokeWidth={2.4} />}
        {done ? t("done.marked") : t("done.mark")}
      </button>
      {mark.error && <p role="alert" className="alert alert-danger" style={{ marginTop: 12 }}>{errorText(mark.error)}</p>}
    </div>
  );
}
