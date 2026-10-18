import { translate } from "./i18n/core";
import type { MessageKey } from "./i18n/core";
import type { AuditRecord } from "./api/types";

/** The actions the server writes to the audit log (ADR-0053); each has a sentence `aud.<action>`. */
const ACTIONS = [
  "content.item.create", "content.item.delete", "content.item.publish", "content.item.reorder", "content.item.revision_publish", "content.item.update",
  "content.module.create", "content.module.delete", "content.module.publish", "content.module.reorder", "content.module.update",
  "course.archive", "course.create", "course.delete", "course.exclusion.restore", "course.group.link", "course.group.unlink", "course.member.put",
  "course.member.remove", "course.owner.transfer", "course.publish", "course.restore", "course.update",
  "grade.publish", "grade.save",
  "group.create", "group.delete", "group.member.add", "group.member.remove", "group.members.remove", "group.members.replace", "group.rename",
  "institution.mode",
  "subgroup.create", "subgroup.delete", "subgroup.member.add", "subgroup.member.remove", "subgroup.rename",
  "term.create", "term.delete", "term.update",
  "user.create", "user.disable", "user.enable", "user.password_reset", "user.rename", "user.role_change",
] as const;

type Action = (typeof ACTIONS)[number];

const known = (action: string): action is Action => (ACTIONS as readonly string[]).includes(action);

const roleLabel = (role: unknown): string | null => (role === "student" || role === "teacher" || role === "admin" ? translate(`role.${role}`) : null);

/** One audit entry in words: who did it, what they did and a detail. The server sends the action and the names (ADR-0053). */
export function auditLine(e: AuditRecord): { who: string; what: string; detail: string } {
  const who = e.actor ? `${e.actor.name} ${e.actor.surname}` : translate("au.system");
  const target = e.resource_label ?? translate("au.gone");
  const what = known(e.action) ? translate(`aud.${e.action}` as MessageKey, { target }) : `${e.action} ${target}`;
  const to = roleLabel(e.meta.to) ?? roleLabel(e.meta.role);
  const from = roleLabel(e.meta.from);
  const detail = from && to ? `${from} → ${to}` : to ?? "";
  return { who, what, detail };
}
