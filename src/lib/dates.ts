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
