import { now } from "./clock";

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

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAYS_LONG = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
export const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const MONTHS_SHORT = MONTHS.map((m) => m.slice(0, 3));

/** The calendar fields of an instant in the institution's zone. */
export function parts(input: string | Date): Parts {
  const d = typeof input === "string" ? new Date(input) : input;
  const p: Record<string, string> = {};
  for (const x of formatter(zone).formatToParts(d)) p[x.type] = x.value;
  return {
    y: Number(p.year),
    m: Number(p.month),
    d: Number(p.day),
    wd: Math.max(0, WEEKDAYS.indexOf(p.weekday ?? "")),
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
  return `${WEEKDAYS[p.wd]} ${p.d} ${MONTHS_SHORT[p.m - 1]}, ${two(p.hh)}:${two(p.mm)}`;
}

/** "Fri 23 Oct" */
export function shortDay(iso: string) {
  const p = parts(iso);
  return `${WEEKDAYS[p.wd]} ${p.d} ${MONTHS_SHORT[p.m - 1]}`;
}

/** "23 Oct" */
export function dayLabel(iso: string) {
  const p = parts(iso);
  return `${p.d} ${MONTHS_SHORT[p.m - 1]}`;
}

/** "23 October" */
export function dayLong(iso: string) {
  const p = parts(iso);
  return `${p.d} ${MONTHS[p.m - 1]}`;
}

/** "Friday 23 October at 23:59" */
export function dueSentence(iso: string) {
  const p = parts(iso);
  return `${WEEKDAYS_LONG[p.wd]} ${p.d} ${MONTHS[p.m - 1]} at ${two(p.hh)}:${two(p.mm)}`;
}

/** "Thursday 29 October" */
export function weekdayDate(iso: string) {
  const p = parts(iso);
  return `${WEEKDAYS_LONG[p.wd]} ${p.d} ${MONTHS[p.m - 1]}`;
}

/** "17 Oct 2026, 15:10" */
export function dateTime(iso: string) {
  const p = parts(iso);
  return `${p.d} ${MONTHS_SHORT[p.m - 1]} ${p.y}, ${two(p.hh)}:${two(p.mm)}`;
}

export function fullDate(d: Date) {
  const p = parts(d);
  return `${WEEKDAYS_LONG[p.wd]}, ${p.d} ${MONTHS[p.m - 1]}`;
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
  if (diff === 0) return "today";
  if (diff === 1) return "tomorrow";
  if (diff === -1) return "yesterday";
  return diff > 0 ? `in ${diff} days` : `${-diff} days ago`;
}

/** "Today", "Tomorrow", "Yesterday", "In 5 days" */
export function relativeCap(iso: string) {
  const r = relative(iso);
  return r.charAt(0).toUpperCase() + r.slice(1);
}

/** "today at 09:41", "yesterday at 19:30" or "12 Oct at 14:02" */
export function sinceLabel(iso: string) {
  const diff = daysFromToday(iso);
  if (diff === 0) return `today at ${timeOf(iso)}`;
  if (diff === -1) return `yesterday at ${timeOf(iso)}`;
  return `${dayLabel(iso)} at ${timeOf(iso)}`;
}

export function greeting(d: Date) {
  const h = parts(d).hh;
  return h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
}

export function initials(name: string, surname: string) {
  return `${name.charAt(0)}${surname.charAt(0)}`.toUpperCase();
}

export function plural(n: number, one: string, many = `${one}s`) {
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
  if (mins < 5) return "Active now";
  if (mins < 60) return `Last active ${plural(mins, "minute")} ago`;
  if (daysFromToday(iso) === 0) return `Last active ${plural(Math.round(mins / 60), "hour")} ago`;
  return `Last active ${dayLong(iso)}`;
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
