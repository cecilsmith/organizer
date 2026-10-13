import { invoke } from "@tauri-apps/api/core";

export type ItemKind = "reminder" | "issue" | "pullRequest" | "draftIssue";
export type Provider = "github" | "gitlab";

export interface Category {
  id: string;
  name: string;
  color: string;
  label: string;
  position: number;
}

export interface Account {
  id: string;
  login: string;
  host: string;
  name: string | null;
  avatarUrl: string | null;
  scopes: string[];
  provider: Provider;
}

export interface Repo {
  id: string;
  accountId: string;
  owner: string;
  name: string;
  url: string;
  defaultCategory: string | null;
  provider: Provider;
}

export interface Project {
  id: string;
  accountId: string;
  owner: string;
  number: number;
  title: string;
  url: string;
  defaultCategory: string | null;
}

export interface ReminderList {
  id: string;
  title: string;
  color: string | null;
  sourceTitle: string | null;
  writable: boolean;
  enabled: boolean;
  defaultCategory: string | null;
}

export interface Label {
  name: string;
  color: string;
}

export interface Item {
  id: string;
  kind: ItemKind;
  /** Issues and PRs: which service they live in. */
  provider: Provider;
  containerId: string;
  containerName: string;
  accountId: string | null;
  title: string;
  body: string | null;
  url: string | null;
  number: number | null;
  completed: boolean;
  due: string | null;
  priority: number;
  labels: Label[];
  assignees: string[];
  status: string | null;
  projectIds: string[];
  updatedAt: string | null;
  /** Start date (`YYYY-MM-DD`), from a GitHub Project start field or iteration. */
  start: string | null;
  /** Where the dates live in a GitHub Project; null when they can't be edited here. */
  projectDates: ProjectDates | null;
  /** GitHub sub-issues: the parent issue's item id. */
  parentId: string | null;
  /** GitHub sub-issues: progress of this issue's own sub-issues. */
  subIssues: { total: number; completed: number } | null;
  categories: string[];
}

export interface ProjectDates {
  projectId: string;
  itemId: string;
  startField: string | null;
  dueField: string | null;
}

export interface SourceStatus {
  sourceId: string;
  label: string;
  lastSynced: string | null;
  error: string | null;
  syncing: boolean;
}

export type RemindersAccess = "granted" | "writeOnly" | "denied" | "restricted" | "notDetermined" | "unsupported";

/** One occurrence of a calendar event (iCloud, Google, Outlook… via macOS). */
export interface CalendarEvent {
  id: string;
  eventId: string;
  occurrence: number;
  calendarId: string;
  calendarName: string;
  color: string | null;
  title: string;
  notes: string | null;
  location: string | null;
  url: string | null;
  /** `YYYY-MM-DDTHH:MM`, or `YYYY-MM-DD` when all-day. */
  start: string;
  /** Timed: end time. All-day: last day (inclusive). */
  end: string;
  allDay: boolean;
  recurring: boolean;
  writable: boolean;
  categories: string[];
}

export interface NewEvent {
  calendarId: string;
  title: string;
  start: string;
  end: string;
  allDay?: boolean;
  location?: string | null;
  notes?: string | null;
  category?: string | null;
}

/** Edits to one event occurrence; omitted fields are left alone, "" clears notes/location. */
export interface EventPatch {
  title?: string;
  notes?: string;
  location?: string;
  start?: string;
  end?: string;
  allDay?: boolean;
  calendarId?: string;
}

export interface Snapshot {
  categories: Category[];
  accounts: Account[];
  repos: Repo[];
  projects: Project[];
  reminderLists: ReminderList[];
  remindersAccess: RemindersAccess;
  eventCalendars: ReminderList[];
  calendarAccess: RemindersAccess;
  events: CalendarEvent[];
  items: Item[];
  sources: SourceStatus[];
}

export interface Candidate {
  id: string;
  owner: string;
  name: string;
  number: number | null;
  url: string;
  description: string | null;
}

export type NewItem =
  | {
      target: "reminder";
      listId: string;
      title: string;
      notes?: string | null;
      due?: string | null;
      priority?: number;
      category?: string | null;
    }
  | {
      target: "issue";
      repoId: string;
      title: string;
      body?: string | null;
      category?: string | null;
      extraLabels?: string[];
      projectId?: string | null;
      assignSelf?: boolean;
    };

/** Editor changes; omitted fields are left alone, "" clears `body`/`due`. */
export interface ItemPatch {
  title?: string;
  body?: string;
  due?: string;
  priority?: number;
  listId?: string;
  labels?: string[];
  assignees?: string[];
}

export interface ItemDetails {
  item: Item;
  labelOptions: Label[];
  assigneeOptions: string[];
}

export const api = {
  snapshot: () => invoke<Snapshot>("get_snapshot"),
  syncNow: () => invoke<void>("sync_now"),

  addAccount: (token: string, host: string, provider: Provider) =>
    invoke<Account>("add_account", { token, host, provider }),
  importGhAccounts: () => invoke<string[]>("import_gh_accounts"),
  removeAccount: (id: string) => invoke<void>("remove_account", { id }),

  repoCandidates: (accountId: string) => invoke<Candidate[]>("repo_candidates", { accountId }),
  projectCandidates: (accountId: string) => invoke<Candidate[]>("project_candidates", { accountId }),
  lookupRepo: (accountId: string, spec: string) => invoke<Candidate>("lookup_repo", { accountId, spec }),
  lookupProject: (accountId: string, spec: string) => invoke<Candidate>("lookup_project", { accountId, spec }),
  addRepo: (accountId: string, candidate: Candidate, category: string | null) =>
    invoke<void>("add_repo", { accountId, candidate, category }),
  addProject: (accountId: string, candidate: Candidate, category: string | null) =>
    invoke<void>("add_project", { accountId, candidate, category }),
  removeSource: (id: string) => invoke<void>("remove_source", { id }),
  setDefaultCategory: (containerId: string, category: string | null) =>
    invoke<void>("set_default_category", { containerId, category }),

  requestRemindersAccess: () => invoke<RemindersAccess>("request_reminders_access"),
  setListEnabled: (id: string, enabled: boolean) => invoke<void>("set_list_enabled", { id, enabled }),

  requestCalendarAccess: () => invoke<RemindersAccess>("request_calendar_access"),
  createEvent: (event: NewEvent) => invoke<CalendarEvent>("create_event", { event }),
  updateEvent: (id: string, patch: EventPatch, fallback?: CalendarEvent) =>
    invoke<CalendarEvent>("update_event", { id, patch, fallback: fallback ?? null }),
  deleteEvent: (id: string, fallback?: CalendarEvent) =>
    invoke<void>("delete_event", { id, fallback: fallback ?? null }),
  openEvent: (id: string) => invoke<void>("open_event", { id }),

  upsertCategory: (category: Category) => invoke<Category>("upsert_category", { category }),
  deleteCategory: (id: string) => invoke<void>("delete_category", { id }),

  createItem: (item: NewItem) => invoke<Item>("create_item", { item }),
  /** `fallback` lets undo work after a sync has dropped the item from the cache. */
  setCompleted: (id: string, completed: boolean, fallback?: Item) =>
    invoke<void>("set_completed", { id, completed, fallback: fallback ?? null }),
  /** Moves/resizes an item's dates. `null` leaves a date alone; `""` clears it. */
  setDates: (id: string, start: string | null, due: string | null, fallback?: Item) =>
    invoke<Item>("set_dates", { id, start, due, fallback: fallback ?? null }),
  /** Undoes creating an item: deletes a reminder, or closes an issue as not planned. */
  discardItem: (id: string) => invoke<void>("discard_item", { id }),
  setCategory: (id: string, category: string | null) => invoke<void>("set_category", { id, category }),
  itemDetails: (id: string) => invoke<ItemDetails>("item_details", { id }),
  updateItem: (id: string, patch: ItemPatch, fallback?: Item) =>
    invoke<Item>("update_item", { id, patch, fallback: fallback ?? null }),
  openItem: (id: string) => invoke<void>("open_item", { id }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}
