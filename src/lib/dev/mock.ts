// Fake backend for working on the UI in a plain browser (`npm run dev`).
// Only loaded in dev builds when the page isn't running inside Tauri.

import { mockIPC } from "@tauri-apps/api/mocks";
import type { CalendarEvent, Category, Item, Snapshot } from "$lib/api";
import { todayStr as day } from "$lib/store.svelte";

const categories: Category[] = [
  { id: "personal", name: "Personal", color: "#4f8cff", label: "personal", position: 0 },
  { id: "business", name: "Business", color: "#f2994a", label: "business", position: 1 },
  { id: "school", name: "School", color: "#9b6dff", label: "school", position: 2 },
  { id: "fun", name: "Fun", color: "#27ae60", label: "fun", position: 3 },
];

/** Project date fields for sample items in "Q4 Roadmap". */
const pd = (n: string) => ({ projectId: "P1", itemId: `PVTI_${n}`, startField: "F_start", dueField: "F_target" });

function item(p: Partial<Item> & Pick<Item, "id" | "title" | "kind">): Item {
  return {
    provider: "github",
    containerId: "",
    containerName: "",
    accountId: null,
    body: null,
    url: null,
    number: null,
    completed: false,
    due: null,
    priority: 0,
    labels: [],
    assignees: [],
    status: null,
    projectIds: [],
    updatedAt: new Date().toISOString(),
    start: null,
    projectDates: null,
    parentId: null,
    subIssues: null,
    categories: [],
    ...p,
  };
}

function event(p: Partial<CalendarEvent> & Pick<CalendarEvent, "id" | "title" | "calendarId" | "start" | "end">): CalendarEvent {
  const cal = calendars.find((c) => c.id === p.calendarId)!;
  return {
    eventId: p.id.slice(3),
    occurrence: 0,
    calendarName: cal.title,
    color: cal.color,
    notes: null,
    location: null,
    url: null,
    allDay: false,
    recurring: false,
    writable: cal.writable,
    categories: cal.defaultCategory ? [cal.defaultCategory] : [],
    ...p,
  };
}

const calendars = [
  { id: "C1", title: "Home", color: "#34c759", sourceTitle: "iCloud", writable: true, enabled: true, defaultCategory: "personal" },
  { id: "C2", title: "Work", color: "#007aff", sourceTitle: "Google (sam@gmail.com)", writable: true, enabled: true, defaultCategory: "business" },
  { id: "C3", title: "Classes", color: "#af52de", sourceTitle: "Exchange (Outlook)", writable: true, enabled: true, defaultCategory: "school" },
  { id: "C4", title: "US Holidays", color: "#ff3b30", sourceTitle: "Other", writable: false, enabled: true, defaultCategory: null },
];

const snap: Snapshot = {
  categories,
  accounts: [
    { id: "octo@github.com", login: "octo", host: "github.com", name: "Octo Cat", avatarUrl: null, scopes: ["repo"], provider: "github", tokenSource: "keychain" },
    { id: "sam@gitlab.com", login: "sam", host: "gitlab.com", name: "Sam", avatarUrl: null, scopes: ["api"], provider: "gitlab", tokenSource: "cli" },
  ],
  repos: [
    { id: "R1", accountId: "octo@github.com", owner: "octo", name: "thesis", url: "", defaultCategory: "school", provider: "github" },
    { id: "R2", accountId: "octo@github.com", owner: "acme", name: "storefront", url: "", defaultCategory: "business", provider: "github" },
    { id: "gl:gitlab.com:42", accountId: "sam@gitlab.com", owner: "robotics-club", name: "rover", url: "", defaultCategory: "fun", provider: "gitlab" },
  ],
  projects: [
    { id: "P1", accountId: "octo@github.com", owner: "acme", number: 3, title: "Q4 Roadmap", url: "", defaultCategory: "business" },
  ],
  reminderLists: [
    { id: "L1", title: "Errands", color: "#ff9500", sourceTitle: "iCloud", writable: true, enabled: true, defaultCategory: "personal" },
    { id: "L2", title: "Homework", color: "#af52de", sourceTitle: "iCloud", writable: true, enabled: true, defaultCategory: "school" },
    { id: "L3", title: "Someday", color: "#34c759", sourceTitle: "iCloud", writable: true, enabled: true, defaultCategory: null },
  ],
  remindersAccess: "granted",
  calendarAccess: "granted",
  eventCalendars: calendars,
  events: [
    event({ id: "ev:E1", title: "Team standup", calendarId: "C2", start: `${day(0)}T09:00`, end: `${day(0)}T09:30`, recurring: true }),
    event({ id: "ev:E2", title: "Lunch with Sam", calendarId: "C1", start: `${day(0)}T12:30`, end: `${day(0)}T13:30`, location: "Café Rio" }),
    event({ id: "ev:E3", title: "EE5351 Lecture", calendarId: "C3", start: `${day(0)}T14:00`, end: `${day(0)}T15:15`, location: "Keller 3-180", recurring: true }),
    event({ id: "ev:E4", title: "Product review", calendarId: "C2", start: `${day(1)}T15:00`, end: `${day(1)}T16:30`, location: "Room 4B", notes: "Bring the Q4 numbers" }),
    event({ id: "ev:E5", title: "Family trip", calendarId: "C1", start: day(5), end: day(7), allDay: true }),
    event({ id: "ev:E6", title: "Indigenous Peoples' Day", calendarId: "C4", start: day(0), end: day(0), allDay: true }),
    event({ id: "ev:E7", title: "Office hours", calendarId: "C3", start: `${day(2)}T10:00`, end: `${day(2)}T11:00` }),
  ],
  items: [
    item({ id: "rem:1", kind: "reminder", title: "Pick up dry cleaning", containerId: "L1", containerName: "Errands", body: "Ticket 4417, they close at 6pm.\nAsk about the stain on the blue shirt.\nPay with card.", due: day(-1), categories: ["personal"] }),
    item({ id: "rem:2", kind: "reminder", title: "Problem set 4", containerId: "L2", containerName: "Homework", due: `${day(0)}T23:59`, priority: 1, body: "Questions 1–7, show work for #5.", categories: ["school"] }),
    item({ id: "rem:3", kind: "reminder", title: "Book climbing gym #fun", containerId: "L3", containerName: "Someday", categories: ["fun"] }),
    item({ id: "rem:4", kind: "reminder", title: "Renew passport", containerId: "L3", containerName: "Someday", body: "#school", due: day(12) }),
    item({ id: "rem:5", kind: "reminder", title: "Dentist", containerId: "L1", containerName: "Errands", due: `${day(1)}T09:30`, categories: ["personal"] }),
    item({ id: "rem:6", kind: "reminder", title: "Study group", containerId: "L2", containerName: "Homework", due: `${day(0)}T10:00`, categories: ["school"] }),
    item({ id: "rem:7", kind: "reminder", title: "Call landlord", containerId: "L1", containerName: "Errands", due: `${day(0)}T10:15`, categories: ["personal"] }),
    item({ id: "rem:8", kind: "reminder", title: "Pay rent", containerId: "L1", containerName: "Errands", due: day(18), priority: 1, categories: ["personal"] }),
    item({ id: "rem:9", kind: "reminder", title: "Concert tickets on sale", containerId: "L3", containerName: "Someday", due: `${day(4)}T12:00`, categories: ["fun"] }),
    item({ id: "gl:gitlab.com:9001", kind: "issue", provider: "gitlab", title: "Calibrate wheel encoders", containerId: "gl:gitlab.com:42", containerName: "robotics-club/rover", number: 17, accountId: "sam@gitlab.com", labels: [{ name: "hardware", color: "e2a03f" }], due: day(6), assignees: ["sam"], categories: ["fun"] }),
    item({ id: "gl:gitlab.com:9002", kind: "issue", provider: "gitlab", title: "Write driver for lidar", containerId: "gl:gitlab.com:42", containerName: "robotics-club/rover", number: 18, accountId: "sam@gitlab.com", categories: ["fun"] }),
    item({ id: "gh:I1", kind: "issue", title: "Write related-work section", containerId: "R1", containerName: "octo/thesis", number: 12, accountId: "octo@github.com", labels: [{ name: "writing", color: "0e8a16" }], due: day(3), categories: ["school"] }),
    item({ id: "gh:I2", kind: "issue", title: "Checkout fails on Safari 18", containerId: "R2", containerName: "acme/storefront", number: 231, accountId: "octo@github.com", labels: [{ name: "bug", color: "d73a4a" }, { name: "business", color: "f2994a" }], status: "In progress", projectIds: ["P1"], assignees: ["octo"], subIssues: { total: 3, completed: 1 }, start: day(-5), due: day(9), projectDates: pd("I2"), categories: ["business"] }),
    item({ id: "gh:I3", kind: "issue", title: "Reproduce on iOS Safari", containerId: "R2", containerName: "acme/storefront", number: 232, accountId: "octo@github.com", parentId: "gh:I2", projectIds: ["P1"], start: day(-3), due: day(1), projectDates: pd("I3"), categories: ["business"] }),
    item({ id: "gh:I4", kind: "issue", title: "Fix payment form validation", containerId: "R2", containerName: "acme/storefront", number: 233, accountId: "octo@github.com", parentId: "gh:I2", projectIds: ["P1"], start: day(2), due: day(8), projectDates: pd("I4"), labels: [{ name: "bug", color: "d73a4a" }], categories: ["business"] }),
    item({ id: "gh:PR1", kind: "pullRequest", title: "Add Apple Pay", containerId: "R2", containerName: "acme/storefront", number: 240, accountId: "octo@github.com", status: "In review", projectIds: ["P1"], start: day(-2), due: day(3), projectDates: pd("PR1"), categories: ["business"] }),
    item({ id: "gh:I5", kind: "issue", title: "Search revamp", containerId: "R2", containerName: "acme/storefront", number: 250, accountId: "octo@github.com", status: "Todo", projectIds: ["P1"], start: day(14), due: day(27), categories: ["business"] }),
    item({ id: "ghpi:D1", kind: "draftIssue", title: "Plan holiday sale", containerId: "P1", containerName: "Q4 Roadmap", accountId: "octo@github.com", status: "Todo", start: day(10), due: day(24), projectDates: pd("D1"), categories: ["business"] }),
  ],
  sources: [
    { sourceId: "reminders", label: "Apple Reminders", lastSynced: new Date().toISOString(), error: null, syncing: false },
    { sourceId: "R1", label: "octo/thesis", lastSynced: new Date().toISOString(), error: null, syncing: false },
    { sourceId: "R2", label: "acme/storefront", lastSynced: new Date().toISOString(), error: null, syncing: false },
    { sourceId: "gl:gitlab.com:42", label: "robotics-club/rover", lastSynced: new Date().toISOString(), error: null, syncing: false },
    { sourceId: "P1", label: "Q4 Roadmap", lastSynced: null, error: "GitHub: missing the `project` scope", syncing: false },
  ],
};

export function installMock() {
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "get_snapshot":
          return structuredClone(snap);
        case "set_completed": {
          const it = snap.items.find((i) => i.id === a.id);
          if (it) it.completed = a.completed as boolean;
          return null;
        }
        case "set_dates": {
          const it = snap.items.find((i) => i.id === a.id);
          if (!it) throw "item not found";
          if (typeof a.start === "string") it.start = a.start || null;
          if (typeof a.due === "string") it.due = a.due || null;
          return structuredClone(it);
        }
        case "create_event": {
          const req = a.event as { calendarId: string; title: string; start: string; end: string; allDay?: boolean; location?: string; notes?: string };
          const e = event({ id: `ev:new${Date.now()}`, ...req, allDay: !!req.allDay, location: req.location || null, notes: req.notes || null });
          snap.events.push(e);
          return e;
        }
        case "update_event": {
          const e = snap.events.find((x) => x.id === a.id);
          if (!e) throw "event not found";
          const p = a.patch as Record<string, unknown>;
          for (const k of ["title", "start", "end", "allDay", "calendarId"]) if (p[k] !== undefined) (e as any)[k] = p[k];
          if (typeof p.notes === "string") e.notes = p.notes || null;
          if (typeof p.location === "string") e.location = p.location || null;
          if (typeof p.calendarId === "string") {
            const cal = calendars.find((c) => c.id === p.calendarId)!;
            Object.assign(e, { calendarName: cal.title, color: cal.color, categories: cal.defaultCategory ? [cal.defaultCategory] : [] });
          }
          return structuredClone(e);
        }
        case "delete_event": {
          snap.events = snap.events.filter((x) => x.id !== a.id);
          return null;
        }
        case "request_calendar_access":
          return "granted";
        case "discard_item": {
          snap.items = snap.items.filter((i) => i.id !== a.id);
          return null;
        }
        case "set_category": {
          const it = snap.items.find((i) => i.id === a.id);
          if (it) it.categories = a.category ? [a.category as string] : [];
          return null;
        }
        case "item_details": {
          const it = snap.items.find((i) => i.id === a.id);
          return new Promise((resolve) =>
            setTimeout(
              () =>
                resolve({
                  item: structuredClone(it),
                  labelOptions: [
                    { name: "bug", color: "d73a4a" },
                    { name: "enhancement", color: "a2eeef" },
                    { name: "writing", color: "0e8a16" },
                    ...categories.map((c) => ({ name: c.label, color: c.color.slice(1) })),
                  ],
                  assigneeOptions: ["octo", "hubot", "monalisa"],
                }),
              300,
            ),
          );
        }
        case "update_item": {
          const it = snap.items.find((i) => i.id === a.id);
          const p = a.patch as Record<string, unknown>;
          if (!it) throw "item not found";
          if (typeof p.title === "string") it.title = p.title;
          if (typeof p.body === "string") it.body = p.body || null;
          if (typeof p.due === "string") it.due = p.due || null;
          if (typeof p.priority === "number") it.priority = p.priority;
          if (typeof p.listId === "string") {
            const list = snap.reminderLists.find((l) => l.id === p.listId)!;
            it.containerId = list.id;
            it.containerName = list.title;
          }
          if (Array.isArray(p.labels)) it.labels = p.labels.map((name: string) => ({ name, color: "ededed" }));
          if (Array.isArray(p.assignees)) it.assignees = p.assignees as string[];
          // Re-derive areas the way the backend would.
          const tags = it.kind === "reminder"
            ? [...`${it.title} ${it.body ?? ""}`.matchAll(/(?:^|\s)#([\w:-]+)/g)].map((m) => m[1].toLowerCase())
            : it.labels.map((l) => l.name.toLowerCase());
          const fromTags = categories.filter((c) => tags.includes(c.label)).map((c) => c.id);
          const listDefault = snap.reminderLists.find((l) => l.id === it.containerId)?.defaultCategory;
          const repoDefault = snap.repos.find((r) => r.id === it.containerId)?.defaultCategory;
          it.categories = fromTags.length ? fromTags : [listDefault ?? repoDefault].filter(Boolean) as string[];
          return structuredClone(it);
        }
        case "create_item": {
          const req = a.item as {
            target: string;
            title: string;
            listId?: string;
            repoId?: string;
            category?: string;
            due?: string | null;
            notes?: string | null;
            body?: string | null;
            priority?: number;
          };
          const list = snap.reminderLists.find((l) => l.id === req.listId);
          const repo = snap.repos.find((r) => r.id === req.repoId);
          const it = item({
            id: `mock:${Date.now()}`,
            kind: req.target === "reminder" ? "reminder" : "issue",
            title: req.title,
            containerId: list?.id ?? repo?.id ?? "",
            containerName: list?.title ?? `${repo?.owner}/${repo?.name}`,
            categories: req.category ? [req.category] : [],
            due: req.due ?? null,
            body: req.notes ?? req.body ?? null,
            priority: req.priority ?? 0,
          });
          snap.items.push(it);
          return it;
        }
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
