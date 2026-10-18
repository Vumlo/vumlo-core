import { now } from "./clock";
import { language, translate, translatePlural } from "./i18n/core";

/** Deadlines are shown in the institution's time zone (Admin, Institution). */
let zone = "Europe/Kyiv";

export function setTimeZone(tz: string) {
  zone = tz;
}

export function timeZone() {
  return zone;
}

type Parts = { y: number; m: number; d: number; wd: number; hh: number; mm: number };

const cache = new Map<string, Intl.DateTimeFormat>();

function formatter(tz: string) {
  let f = cache.get(tz);
  if (!f) {
    f = new Intl.DateTimeFormat("en-GB", {
      timeZone: tz,
      year: "numeric",
      month: "numeric",
      day: "numeric",
      weekday: "short",
      hour: "numeric",
      minute: "numeric",
      hourCycle: "h23",
    });
    cache.set(tz, f);
  }
  return f;
}

/** English short names, in the order Intl reports them: only used to find the weekday number. */
const WEEKDAY_KEYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

const NAMES = {
  en: {
    weekdays: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    weekdaysLong: ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"],
    months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    monthsOf: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"],
    monthsShort: ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
  },
  uk: {
    weekdays: ["пн", "вт", "ср", "чт", "пт", "сб", "нд"],
    weekdaysLong: ["понеділок", "вівторок", "середа", "четвер", "пʼятниця", "субота", "неділя"],
    months: ["січень", "лютий", "березень", "квітень", "травень", "червень", "липень", "серпень", "вересень", "жовтень", "листопад", "грудень"],
    monthsOf: ["січня", "лютого", "березня", "квітня", "травня", "червня", "липня", "серпня", "вересня", "жовтня", "листопада", "грудня"],
    monthsShort: ["січ", "лют", "бер", "кві", "тра", "чер", "лип", "сер", "вер", "жов", "лис", "гру"],
  },
} as const;

const names = () => NAMES[language()];
const cap = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

/** 1 to 12. The month on its own: a heading, "October 2026". */
export const monthName = (m: number) => cap(names().months[m - 1] ?? "");
/** The month after a day number: "23 October", in Ukrainian "23 жовтня". */
export const monthOf = (m: number) => names().monthsOf[m - 1] ?? "";
export const monthShort = (m: number) => cap(names().monthsShort[m - 1] ?? "");
/** 0 is Monday. */
export const weekdayShort = (wd: number) => cap(names().weekdays[wd] ?? "");
export const weekdayLong = (wd: number) => cap(names().weekdaysLong[wd] ?? "");

/** The calendar fields of an instant in the institution's zone. */
export function parts(input: string | Date): Parts {
  const d = typeof input === "string" ? new Date(input) : input;
  const p: Record<string, string> = {};
  for (const x of formatter(zone).formatToParts(d)) p[x.type] = x.value;
  return {
    y: Number(p.year),
    m: Number(p.month),
    d: Number(p.day),
    wd: Math.max(0, WEEKDAY_KEYS.indexOf(p.weekday ?? "")),
    hh: Number(p.hour),
    mm: Number(p.minute),
  };
}

const two = (n: number) => String(n).padStart(2, "0");

export function timeOf(iso: string | Date) {
  const p = parts(iso);
  return `${two(p.hh)}:${two(p.mm)}`;
}

/** "Fri 23 Oct, 23:59" */
export function dueLabel(iso: string) {
  const p = parts(iso);
  return `${weekdayShort(p.wd)} ${p.d} ${monthShort(p.m)}, ${two(p.hh)}:${two(p.mm)}`;
}

/** "Fri 23 Oct" */
export function shortDay(iso: string) {
  const p = parts(iso);
  return `${weekdayShort(p.wd)} ${p.d} ${monthShort(p.m)}`;
}

/** "23 Oct" */
export function dayLabel(iso: string) {
  const p = parts(iso);
  return `${p.d} ${monthShort(p.m)}`;
}

/** "23 October" */
export function dayLong(iso: string) {
  const p = parts(iso);
  return `${p.d} ${monthOf(p.m)}`;
}

/** "Friday 23 October at 23:59" */
export function dueSentence(iso: string) {
  const p = parts(iso);
  return translate("date.dueSentence", { weekday: weekdayLong(p.wd), day: p.d, month: monthOf(p.m), time: `${two(p.hh)}:${two(p.mm)}` });
}

/** "Thursday 29 October" */
export function weekdayDate(iso: string) {
  const p = parts(iso);
  return `${weekdayLong(p.wd)} ${p.d} ${monthOf(p.m)}`;
}

/** "17 Oct 2026, 15:10" */
export function dateTime(iso: string) {
  const p = parts(iso);
  return `${p.d} ${monthShort(p.m)} ${p.y}, ${two(p.hh)}:${two(p.mm)}`;
}

/** "Saturday, 17 October" */
export function fullDate(d: Date) {
  const p = parts(d);
  return `${weekdayLong(p.wd)}, ${p.d} ${monthOf(p.m)}`;
}

/** The weekday and day for a "YYYY-MM-DD" key: "Saturday, 17 October". */
export function dayKeyLabel(key: string) {
  const [y = 1970, m = 1, d = 1] = key.split("-").map(Number);
  return `${weekdayLong((new Date(Date.UTC(y, m - 1, d)).getUTCDay() + 6) % 7)}, ${d} ${monthOf(m)}`;
}

/** Whole calendar days from today (in the institution zone) to the instant. */
export function daysFromToday(iso: string) {
  const a = parts(now());
  const b = parts(iso);
  return Math.round((Date.UTC(b.y, b.m - 1, b.d) - Date.UTC(a.y, a.m - 1, a.d)) / 86_400_000);
}

/** "in 6 days", "tomorrow", "today", "3 days ago" */
export function relative(iso: string) {
  const diff = daysFromToday(iso);
  if (diff === 0) return translate("date.today");
  if (diff === 1) return translate("date.tomorrow");
  if (diff === -1) return translate("date.yesterday");
  return diff > 0 ? translatePlural("date.inDays", diff) : translatePlural("date.daysAgo", -diff);
}

/** "Today", "Tomorrow", "Yesterday", "In 5 days" */
export function relativeCap(iso: string) {
  return cap(relative(iso));
}

/** "today at 09:41", "yesterday at 19:30" or "12 Oct at 14:02" */
export function sinceLabel(iso: string) {
  const diff = daysFromToday(iso);
  if (diff === 0) return translate("date.todayAt", { time: timeOf(iso) });
  if (diff === -1) return translate("date.yesterdayAt", { time: timeOf(iso) });
  return translate("date.dayAt", { day: dayLabel(iso), time: timeOf(iso) });
}

export function greeting(d: Date) {
  const h = parts(d).hh;
  return translate(h < 12 ? "greeting.morning" : h < 18 ? "greeting.afternoon" : "greeting.evening");
}

export function initials(name: string, surname: string) {
  return `${name.charAt(0)}${surname.charAt(0)}`.toUpperCase();
}

/** Ukrainian forms of the nouns counted in the UI: one, few (2 to 4), many (0, 5 and up). */
const UK_UNITS: Record<string, [string, string, string]> = {
  student: ["студент", "студенти", "студентів"],
  "former student": ["колишній студент", "колишні студенти", "колишніх студентів"],
  assistant: ["асистент", "асистенти", "асистентів"],
  section: ["розділ", "розділи", "розділів"],
  item: ["матеріал", "матеріали", "матеріалів"],
  lecture: ["лекція", "лекції", "лекцій"],
  assignment: ["завдання", "завдання", "завдань"],
  draft: ["чернетка", "чернетки", "чернеток"],
  "draft grade": ["чернетка оцінки", "чернетки оцінок", "чернеток оцінок"],
  "graded item": ["оцінене завдання", "оцінені завдання", "оцінених завдань"],
  course: ["курс", "курси", "курсів"],
  team: ["команда", "команди", "команд"],
  option: ["варіант", "варіанти", "варіантів"],
  teaching: ["викладач", "викладачі", "викладачів"],
  "draft item": ["чернетка матеріалу", "чернетки матеріалів", "чернеток матеріалів"],
  day: ["день", "дні", "днів"],
  minute: ["хвилина", "хвилини", "хвилин"],
  hour: ["година", "години", "годин"],
};

/** "3 students". Pass `many` when the English plural is not the word plus "s". */
export function plural(n: number, one: string, many = `${one}s`) {
  const uk = language() === "uk" ? UK_UNITS[one] : undefined;
  if (uk) {
    const rule = new Intl.PluralRules("uk").select(n);
    return `${n} ${rule === "one" ? uk[0] : rule === "few" ? uk[1] : uk[2]}`;
  }
  return `${n} ${n === 1 ? one : many}`;
}

/** The UTC offset label of the zone, "UTC+3". */
export function offsetLabel(at: Date = now()) {
  const f = new Intl.DateTimeFormat("en-GB", { timeZone: zone, timeZoneName: "shortOffset" });
  const name = f.formatToParts(at).find((x) => x.type === "timeZoneName")?.value ?? "GMT";
  return name.replace("GMT", "UTC");
}

/** "2026-10-17" key for grouping by day in the zone. */
export function dayKey(iso: string | Date) {
  const p = parts(iso);
  return `${p.y}-${two(p.m)}-${two(p.d)}`;
}

/** "Active now", "Last active 2 hours ago" or "Last active 12 October". */
export function lastActive(iso: string): string {
  const mins = Math.round((now().getTime() - new Date(iso).getTime()) / 60_000);
  if (mins < 5) return translate("date.activeNow");
  if (mins < 60) return translate("date.lastActiveAgo", { ago: translatePlural("date.minutesAgo", mins) });
  if (daysFromToday(iso) === 0) return translate("date.lastActiveAgo", { ago: translatePlural("date.hoursAgo", Math.round(mins / 60)) });
  return translate("date.lastActiveOn", { day: dayLong(iso) });
}

/** An ISO instant from a `datetime-local` value ("2026-10-23T23:59") read in the institution's zone. */
export function localToIso(local: string): string {
  const wall = new Date(`${local.length === 16 ? `${local}:00` : local}Z`);
  const off = (d: Date) => {
    const p: Record<string, number> = {};
    for (const x of new Intl.DateTimeFormat("en-GB", { timeZone: zone, year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric", hourCycle: "h23" }).formatToParts(d)) if (x.type !== "literal") p[x.type] = Number(x.value);
    return Date.UTC(p.year!, p.month! - 1, p.day!, p.hour!, p.minute!, p.second!) - Math.floor(d.getTime() / 1000) * 1000;
  };
  let t = wall.getTime() - off(wall);
  t = wall.getTime() - off(new Date(t));
  return new Date(t).toISOString();
}

/** The `datetime-local` value of an instant in the institution's zone. */
export function isoToLocal(iso: string): string {
  const p = parts(iso);
  return `${p.y}-${two(p.m)}-${two(p.d)}T${two(p.hh)}:${two(p.mm)}`;
}
