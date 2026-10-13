// Local-date helpers for the calendar. Due dates are local, floating values:
// `2026-10-12` (all day) or `2026-10-12T14:30` (timed), never UTC.

export type Ymd = string;

export function ymd(d: Date): Ymd {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

/** Parses `YYYY-MM-DD` (or the date part of a due string) as local midnight. */
export function parseYmd(s: string): Date {
  const [y, m, d] = s.slice(0, 10).split("-").map(Number);
  return new Date(y, m - 1, d);
}

export function addDays(d: Date, n: number): Date {
  const x = new Date(d);
  x.setDate(x.getDate() + n);
  return x;
}

export function addMonths(d: Date, n: number): Date {
  const x = new Date(d.getFullYear(), d.getMonth() + n, 1);
  // Keep the day when it exists in the target month.
  x.setDate(Math.min(d.getDate(), new Date(x.getFullYear(), x.getMonth() + 1, 0).getDate()));
  return x;
}

export function startOfWeek(d: Date, weekStart: number): Date {
  const diff = (d.getDay() - weekStart + 7) % 7;
  return addDays(new Date(d.getFullYear(), d.getMonth(), d.getDate()), -diff);
}

/** The 6×7 grid of days shown for the month containing `d`. */
export function monthGrid(d: Date, weekStart: number): Date[] {
  const first = startOfWeek(new Date(d.getFullYear(), d.getMonth(), 1), weekStart);
  return Array.from({ length: 42 }, (_, i) => addDays(first, i));
}

export function weekDays(d: Date, weekStart: number): Date[] {
  const first = startOfWeek(d, weekStart);
  return Array.from({ length: 7 }, (_, i) => addDays(first, i));
}

/** Minutes after midnight for a timed due string, or null for all-day. */
export function dueMinutes(due: string): number | null {
  if (due.length <= 10) return null;
  const [h, m] = due.slice(11, 16).split(":").map(Number);
  return h * 60 + (m || 0);
}

export function withTime(day: Ymd, minutes: number | null): string {
  if (minutes === null) return day;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return `${day}T${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

export function formatTime(minutes: number): string {
  const d = new Date(2000, 0, 1, Math.floor(minutes / 60), minutes % 60);
  return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

/** Compact time for tight spaces: "10a", "10:15a" (12-hour locales) or "10:15". */
export function formatTimeShort(minutes: number): string {
  const parts = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).formatToParts(
    new Date(2000, 0, 1, Math.floor(minutes / 60), minutes % 60),
  );
  const get = (t: string) => parts.find((p) => p.type === t)?.value ?? "";
  const period = get("dayPeriod");
  if (!period) return `${get("hour")}:${get("minute")}`;
  const m = get("minute");
  return `${get("hour")}${m === "00" ? "" : `:${m}`}${period.trim()[0].toLowerCase()}`;
}

export function formatHour(h: number): string {
  return new Date(2000, 0, 1, h).toLocaleTimeString(undefined, { hour: "numeric" });
}

export function weekdayNames(weekStart: number, style: "short" | "narrow" | "long" = "short"): string[] {
  // 2023-01-01 was a Sunday.
  return Array.from({ length: 7 }, (_, i) =>
    new Date(2023, 0, 1 + ((i + weekStart) % 7)).toLocaleDateString(undefined, { weekday: style }),
  );
}

// ---- events -------------------------------------------------------------------

/** Parses `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM` as a local Date. */
export function parseLocal(s: string): Date {
  const d = parseYmd(s);
  const m = dueMinutes(s);
  if (m !== null) d.setHours(Math.floor(m / 60), m % 60);
  return d;
}

export function toLocalString(d: Date, withClock = true): string {
  return withClock ? withTime(ymd(d), d.getHours() * 60 + d.getMinutes()) : ymd(d);
}

interface Span {
  start: string;
  end: string;
  allDay: boolean;
}

/** Days an event touches. A timed event ending exactly at midnight doesn't touch that day. */
export function eventDays(e: Span): Ymd[] {
  const first = parseYmd(e.start);
  let last = parseYmd(e.end);
  if (!e.allDay && dueMinutes(e.end) === 0 && e.end.slice(0, 10) > e.start.slice(0, 10)) last = addDays(last, -1);
  const out: Ymd[] = [];
  for (let d = first; d <= last && out.length < 366; d = addDays(d, 1)) out.push(ymd(d));
  return out;
}

/** The minutes of `day` a timed event covers, or null if it doesn't touch it. */
export function segmentOn(e: Span, day: Ymd): { start: number; end: number } | null {
  if (e.allDay) return null;
  const dayStart = parseYmd(day).getTime();
  const dayEnd = dayStart + 86_400_000;
  const s = parseLocal(e.start).getTime();
  const en = parseLocal(e.end).getTime();
  if (en <= dayStart || s >= dayEnd) return null;
  return {
    start: Math.round((Math.max(s, dayStart) - dayStart) / 60_000),
    end: Math.round((Math.min(en, dayEnd) - dayStart) / 60_000),
  };
}

/**
 * Where an event lands when dropped on `day`: at `minutes` (keeping its
 * length; all-day events become an hour long), or as an all-day event when
 * `minutes` is null (keeping how many days it spans).
 */
export function movedEvent(e: Span, day: Ymd, minutes: number | null): Span {
  if (minutes === null) {
    const days = e.allDay ? Math.round((parseYmd(e.end).getTime() - parseYmd(e.start).getTime()) / 86_400_000) : 0;
    return { start: day, end: ymd(addDays(parseYmd(day), days)), allDay: true };
  }
  const length = e.allDay ? 60 : Math.max(15, Math.round((parseLocal(e.end).getTime() - parseLocal(e.start).getTime()) / 60_000));
  const start = parseLocal(withTime(day, minutes));
  const end = new Date(start.getTime() + length * 60_000);
  return { start: toLocalString(start), end: toLocalString(end), allDay: false };
}
