import { listen } from "@tauri-apps/api/event";
import {
  api,
  errorText,
  type CalendarEvent,
  type EventPatch,
  type Item,
  type ItemPatch,
  type Snapshot,
} from "./api";
import { addDays, addMonths, eventDays, parseYmd, ymd } from "./dates";

export type SourceFilter = "all" | "reminders" | "github";
export type GroupBy = "source" | "due";
export type SortBy = "due" | "priority" | "title" | "updated" | "container" | "manual";

/** Whether reminder notes show under every row, or only the selected one. */
export type ShowNotes = "always" | "selected";
export type View = "list" | "calendar" | "gantt" | "completed";
export type GanttZoom = "day" | "week" | "month";
export type GanttGroup = "source" | "area";
export type CalView = "month" | "week" | "day";

/** One undoable change: how to reverse it, and how to apply it again. */
export interface UndoEntry {
  /** Describes the original change, e.g. `complete “Buy milk”`. */
  label: string;
  undo: () => Promise<unknown>;
  redo: () => Promise<unknown>;
}

const UNDO_LIMIT = 50;

/** Completed items from one day, for the Completed view. */
export interface DoneGroup {
  /** Group key for `store.collapsed`. */
  key: string;
  /** `YYYY-MM-DD`, or "" when the source didn't say when it was finished. */
  day: string;
  title: string;
  items: Item[];
}

/** One block of the Today view: today's schedule, what's due, what slipped. */
export interface TodaySection {
  /** Group key for `store.collapsed`. */
  key: string;
  title: string;
  icon: string;
  /** A section holds either events or items, never both. */
  events: CalendarEvent[];
  items: Item[];
}

export interface Toast {
  id: number;
  text: string;
  kind: "error" | "info";
  action?: { label: string; run: () => void };
}

/** Completed items stay on screen this long so a mis-click can be undone. */
const LINGER_MS = 4000;
/** How far back the Completed view reaches; matches `COMPLETED_DAYS` in app.rs. */
export const COMPLETED_DAYS = 30;
const FOCUS_SYNC_AFTER_MS = 30_000;

const PREFS_KEY = "organizer.prefs";

interface Prefs {
  tab: string;
  groupBy: GroupBy;
  sortBy: SortBy;
  /** Keys of the list-view groups that are collapsed. */
  collapsed: string[];
  /** Hand-arranged row order, by list-view group key. */
  order: Record<string, string[]>;

  sourceFilter: SourceFilter;
  showNotes: ShowNotes;
  view: View;
  calView: CalView;
  weekStart: number;
  showUnscheduled: boolean;
  ganttZoom: GanttZoom;
  ganttGroup: GanttGroup;
  showEvents: boolean;
}

function loadPrefs(): Partial<Prefs> {
  try {
    return JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}");
  } catch {
    return {};
  }
}

class Store {
  snap = $state<Snapshot | null>(null);
  /** "today", "all", "inbox" or a category id. A fresh install opens on Today. */
  tab = $state<string>("today");
  query = $state("");
  sourceFilter = $state<SourceFilter>("all");
  groupBy = $state<GroupBy>("source");
  sortBy = $state<SortBy>("due");
  /** Collapsed list-view groups, by group key. */
  collapsed = $state<Record<string, boolean>>({});
  /**
   * Hand-arranged order for the "Manual" sort, by list-view group key. Neither
   * EventKit nor GitHub has a field for this, so it lives here.
   */
  order = $state<Record<string, string[]>>({});

  showNotes = $state<ShowNotes>("selected");
  view = $state<View>("list");
  calView = $state<CalView>("month");
  /** The day the calendar is showing (any day within the visible month/week). */
  calDate = $state<string>(ymd(new Date()));
  /** 0 = Sunday, 1 = Monday. */
  weekStart = $state(0);
  showUnscheduled = $state(true);
  ganttZoom = $state<GanttZoom>("week");
  ganttGroup = $state<GanttGroup>("source");
  /** Show calendar events in the Calendar views. */
  showEvents = $state(true);
  /** Event open in the event editor. */
  editingEvent = $state<CalendarEvent | null>(null);
  /** Set to prefill the quick-add due date (`YYYY-MM-DD` or with `THH:MM`). */
  quickAddDue = $state<string | null>(null);
  settingsOpen = $state(false);
  settingsSection = $state<string>("accounts");
  toasts = $state<Toast[]>([]);
  /** Item whose "move to area" menu is open. */
  menuFor = $state<string | null>(null);
  /** The focused row: what the keyboard acts on. */
  selectedId = $state<string | null>(null);
  /** Where a range selection starts. The focused row moves; this stays put. */
  anchorId = $state<string | null>(null);

  /**
   * The multi-selection, by item id. Empty until you Cmd-click, Shift-click or
   * press Cmd+A, so ordinary clicking and the keyboard behave as they always did.
   */
  picked = $state<Record<string, true>>({});

  /** Item open in the editor panel. */
  editing = $state<Item | null>(null);

  /** Optimistic state, keyed by item id, layered over the snapshot. */
  private overrides = $state<Record<string, Partial<Item>>>({});
  /** Recently completed items that should remain visible even if a sync drops them. */
  private lingering = $state<Record<string, Item>>({});

  /** Most recent change last. Both stacks live for the session only. */
  private undoStack: UndoEntry[] = [];
  /** Changes that were undone, most recent last; cleared by any new change. */
  private redoStack: UndoEntry[] = [];
  private undoing = false;

  private refreshing = false;
  private refreshAgain = false;
  private lastSync = 0;
  private toastSeq = 0;

  constructor() {
    const p = loadPrefs();
    if (p.tab) {
      this.tab = p.tab;
      if (p.tab !== "today") this.lastTab = p.tab;
    }
    if (p.groupBy) this.groupBy = p.groupBy;
    if (p.sortBy && p.sortBy in SORTS) this.sortBy = p.sortBy;
    if (Array.isArray(p.collapsed)) this.collapsed = Object.fromEntries(p.collapsed.map((k) => [k, true]));
    if (p.order && typeof p.order === "object") this.order = p.order;

    if (p.showNotes) this.showNotes = p.showNotes;
    if (p.view) this.view = p.view;
    if (p.calView) this.calView = p.calView;
    if (p.weekStart === 0 || p.weekStart === 1) this.weekStart = p.weekStart;
    if (typeof p.showUnscheduled === "boolean") this.showUnscheduled = p.showUnscheduled;
    if (p.ganttZoom) this.ganttZoom = p.ganttZoom;
    if (p.ganttGroup) this.ganttGroup = p.ganttGroup;
    if (typeof p.showEvents === "boolean") this.showEvents = p.showEvents;
    if (p.sourceFilter) this.sourceFilter = p.sourceFilter;
  }

  savePrefs() {
    try {
      localStorage.setItem(
        PREFS_KEY,
        JSON.stringify({
          tab: this.tab,
          groupBy: this.groupBy,
          sortBy: this.sortBy,
          collapsed: Object.keys(this.collapsed).filter((k) => this.collapsed[k]),
          order: this.order,

          sourceFilter: this.sourceFilter,
          showNotes: this.showNotes,
          view: this.view,
          calView: this.calView,
          weekStart: this.weekStart,
          showUnscheduled: this.showUnscheduled,
          ganttZoom: this.ganttZoom,
          ganttGroup: this.ganttGroup,
          showEvents: this.showEvents,
        } satisfies Prefs),
      );
    } catch {
      /* storage unavailable; prefs are a convenience */
    }
  }

  async init() {
    await listen("state-changed", () => this.refresh());
    await this.refresh();
    this.lastSync = Date.now();
    window.addEventListener("focus", () => {
      if (Date.now() - this.lastSync > FOCUS_SYNC_AFTER_MS) this.sync();
    });
  }

  /** Re-fetches the snapshot; bursts of change events collapse into at most one extra fetch. */
  async refresh() {
    if (this.refreshing) {
      this.refreshAgain = true;
      return;
    }
    this.refreshing = true;
    try {
      do {
        this.refreshAgain = false;
        this.snap = await api.snapshot();
      } while (this.refreshAgain);
    } catch (e) {
      this.toast(errorText(e));
    } finally {
      this.refreshing = false;
    }
  }

  sync() {
    this.lastSync = Date.now();
    api.syncNow().catch((e) => this.toast(errorText(e)));
  }

  get syncing() {
    return this.snap?.sources.some((s) => s.syncing) ?? false;
  }

  get sourceErrors() {
    return this.snap?.sources.filter((s) => s.error) ?? [];
  }

  get categories() {
    return this.snap?.categories ?? [];
  }

  /** Snapshot items with optimistic edits applied, plus lingering completions. */
  items: Item[] = $derived.by(() => {
    const base = this.snap?.items ?? [];
    const seen = new Set<string>();
    const out: Item[] = [];
    for (const it of base) {
      seen.add(it.id);
      const o = this.overrides[it.id];
      const merged = o ? { ...it, ...o } : it;
      // Completed items linger briefly for undo, then drop out even if the
      // cache still has them (GitHub only re-syncs every couple of minutes).
      if (merged.completed && !this.lingering[it.id]) continue;
      out.push(merged);
    }
    for (const [id, it] of Object.entries(this.lingering)) {
      if (!seen.has(id)) out.push({ ...it, ...this.overrides[id] });
    }
    return out;
  });

  /**
   * Items the backend has as done, newest first: the Completed view's contents.
   * The cache keeps the last 30 days, so this is a window, not a full history.
   */
  completedItems: Item[] = $derived.by(() => {
    const out: Item[] = [];
    for (const it of this.snap?.items ?? []) {
      const o = this.overrides[it.id];
      const merged = o ? { ...it, ...o } : it;
      if (merged.completed) out.push(merged);
    }
    return out.sort((a, b) => (b.completedAt ?? "").localeCompare(a.completedAt ?? ""));
  });

  /** Completed items under the current area tab and search. */
  visibleDone: Item[] = $derived.by(() =>
    this.completedItems.filter((it) => this.inTab(it, this.tab) && this.matches(it)),
  );

  /**
   * Completed items grouped by the local day they were finished, newest first.
   * Items the source gave no completion time for land under "Earlier".
   */
  doneGroups: DoneGroup[] = $derived.by(() => {
    const groups = new Map<string, DoneGroup>();
    for (const it of this.visibleDone) {
      const day = it.completedAt ? ymd(new Date(it.completedAt)) : "";
      let g = groups.get(day);
      if (!g) {
        g = { key: `done:${day}`, day, title: day ? formatDue(day) : "Earlier", items: [] };
        groups.set(day, g);
      }
      g.items.push(it);
    }
    // `visibleDone` is already newest first, so the days come out in order.
    return [...groups.values()];
  });

  /** Open-item counts per tab id ("today", "all", "inbox", or a category id). */
  counts: Record<string, number> = $derived.by(() => {
    const c: Record<string, number> = { today: 0, all: 0, inbox: 0 };
    for (const it of this.items) {
      if (it.completed || !this.matchesSource(it)) continue;
      c.all++;
      // The Today badge counts what needs doing, so events aren't in it.
      const bucket = dueBucket(it.due);
      if (bucket === "overdue" || bucket === "today") c.today++;
      if (it.categories.length === 0) c.inbox++;
      for (const cat of it.categories) c[cat] = (c[cat] ?? 0) + 1;
    }
    return c;
  });

  /** Optimistic event edits (drag/resize), layered over the snapshot. */
  private eventOverrides = $state<Record<string, Partial<CalendarEvent>>>({});

  /**
   * Every area's events, with optimistic edits and the search applied; the
   * Reminders/GitHub source filters hide them.
   */
  private searchedEvents: CalendarEvent[] = $derived.by(() => {
    if (this.sourceFilter !== "all") return [];
    const q = this.query.trim().toLowerCase();
    return (this.snap?.events ?? [])
      .map((e) => (this.eventOverrides[e.id] ? { ...e, ...this.eventOverrides[e.id] } : e))
      .filter(
        (e) =>
          !q ||
          q.split(/\s+/).every((t) =>
            [e.title, e.location, e.notes, e.calendarName].some((f) => f?.toLowerCase().includes(t)),
          ),
      );
  });

  /** Events for the Calendar views: the current area tab applies, and the eye toggle. */
  visibleEvents: CalendarEvent[] = $derived(
    this.showEvents ? this.searchedEvents.filter((e) => this.inTab(e, this.tab)) : [],
  );

  visible: Item[] = $derived.by(() =>
    this.items.filter((it) => this.inTab(it, this.tab) && this.matches(it)).sort(SORTS[this.sortBy]),
  );

  // ---- Today ----------------------------------------------------------------
  // The Today view answers "what am I doing today" across every area, so these
  // lists deliberately ignore the area tab. Search and the source filters still
  // apply, since their controls sit in the header on this screen too.

  /** Events touching today, all-day ones first, then in start order. */
  todayEvents: CalendarEvent[] = $derived.by(() => {
    const today = todayStr();
    return this.searchedEvents
      .filter((e) => eventDays(e).includes(today))
      .sort((a, b) => Number(b.allDay) - Number(a.allDay) || a.start.localeCompare(b.start));
  });

  /** Items due today, in every area. A time that has passed counts as overdue. */
  dueToday: Item[] = $derived.by(() =>
    this.items.filter((it) => this.matches(it) && dueBucket(it.due) === "today").sort(SORTS.due),
  );

  /** Items past their due date, in every area, longest overdue first. */
  overdue: Item[] = $derived.by(() =>
    this.items.filter((it) => this.matches(it) && dueBucket(it.due) === "overdue").sort(SORTS.due),
  );

  /** The Today view's blocks, in screen order; empty ones are left out. */
  todaySections: TodaySection[] = $derived(
    [
      { key: "today:events", title: "Schedule", icon: "calendar", events: this.todayEvents, items: [] },
      { key: "today:due", title: "Due today", icon: "flag", events: [], items: this.dueToday },
      { key: "today:overdue", title: "Overdue", icon: "alert", events: [], items: this.overdue },
    ].filter((s) => s.events.length || s.items.length),
  );

  /** The tab to come back to when leaving Today; Today itself is never it. */
  private lastTab = "all";

  setTab(tab: string) {
    if (tab !== "today") this.lastTab = tab;
    this.tab = tab;
    this.selectedId = null;
    this.menuFor = null;
    this.clearPicked();
    this.savePrefs();
  }

  /** Today is a tab rather than a view, so V steps back to the tab you came from. */
  leaveToday() {
    this.setTab(this.lastTab === "today" ? "all" : this.lastTab);
  }

  setSort(sortBy: SortBy) {
    this.sortBy = sortBy;
    this.savePrefs();
  }

  /** Folds a list-view group away; the choice is remembered across restarts. */
  toggleGroup(key: string) {
    if (this.collapsed[key]) delete this.collapsed[key];
    else this.collapsed[key] = true;
    this.savePrefs();
  }

  // ---- multi-selection ------------------------------------------------------

  /** The selected items, open or completed. Ids with no item left are ignored. */
  pickedItems: Item[] = $derived.by(() => {
    const out = new Map<string, Item>();
    for (const it of [...this.items, ...this.completedItems]) {
      if (this.picked[it.id] && !out.has(it.id)) out.set(it.id, it);
    }
    return [...out.values()];
  });

  get pickedCount() {
    return Object.keys(this.picked).length;
  }

  /** A plain click or an arrow key: one focused row and no multi-selection. */
  focus(id: string | null) {
    this.selectedId = id;
    this.anchorId = id;
    this.menuFor = null;
    this.clearPicked();
  }

  /** Cmd-click: adds or removes one row, and moves the range anchor to it. */
  togglePicked(id: string) {
    if (this.picked[id]) delete this.picked[id];
    else this.picked[id] = true;
    this.selectedId = id;
    this.anchorId = id;
    this.menuFor = null;
  }

  /**
   * Shift-click or Shift+arrow: every row between the anchor and `id`. The
   * anchor stays where the range began, so the selection grows and shrinks.
   */
  pickRange(rowIds: string[], id: string) {
    const to = rowIds.indexOf(id);
    if (to < 0) return;
    if (!this.anchorId || !rowIds.includes(this.anchorId)) this.anchorId = this.selectedId ?? id;
    const from = rowIds.indexOf(this.anchorId);
    const [lo, hi] = from < 0 ? [to, to] : [Math.min(from, to), Math.max(from, to)];
    this.pick(rowIds.slice(lo, hi + 1));
    this.selectedId = id;
  }

  /** Replaces the selection, e.g. from Cmd+A. */
  pick(ids: string[]) {
    this.picked = Object.fromEntries(ids.map((id) => [id, true as const]));
    this.menuFor = null;
  }

  clearPicked() {
    this.picked = {};
  }

  /** Whether all of `ids` are already selected, which is how Cmd+A widens. */
  allPicked(ids: string[]) {
    return ids.length > 0 && ids.every((id) => this.picked[id]);
  }

  /**
   * Runs `change` over every item, carrying on when one fails, and reports the
   * failures once. Returns what worked, so a batch's undo only reverses those.
   */
  private async eachItem(items: Item[], change: (it: Item) => Promise<unknown>): Promise<Item[]> {
    const ok: Item[] = [];
    const failed: Item[] = [];
    let reason = "";
    for (const it of items) {
      try {
        await change(it);
        ok.push(it);
      } catch (e) {
        failed.push(it);
        if (!reason) reason = errorText(e);
      }
    }
    if (failed.length) {
      const what = failed.length === 1 ? `“${failed[0].title}”` : `${failed.length} of ${items.length} items`;
      this.toast(`Couldn't change ${what}: ${reason}`);
    }
    return ok;
  }

  /** Completes or reopens every selected item, under one undo entry. */
  async completePicked(completed: boolean) {
    const targets = this.pickedItems.filter((it) => it.kind !== "pullRequest" && it.completed !== completed);
    if (!targets.length) return;
    // `toggle` flips whatever it is given, so hand it the opposite each time.
    const flip = (want: boolean) => (it: Item) => this.toggle({ ...it, completed: !want }, false);
    const ok = await this.eachItem(targets, flip(completed));
    if (ok.length) {
      this.record(
        `${completed ? "complete" : "reopen"} ${plural(ok.length)}`,
        () => this.eachItem(ok, flip(!completed)),
        () => this.eachItem(ok, flip(completed)),
      );
    }
    this.clearPicked();
    await this.refresh();
  }

  /** Moves every selected item to one area, under one undo entry. */
  async categorizePicked(category: string | null) {
    const targets = this.pickedItems;
    if (!targets.length) return;
    const restores = new Map(targets.map((it) => [it.id, restorePatch(it)] as const));
    const ok = await this.eachItem(targets, (it) => this.setCategory(it, category, false));
    if (ok.length) {
      const name = this.categories.find((c) => c.id === category)?.name ?? "Inbox";
      this.record(
        `move ${plural(ok.length)} to ${name}`,
        () => this.eachItem(ok, (it) => api.updateItem(it.id, restores.get(it.id)!, it)),
        () => this.eachItem(ok, (it) => api.setCategory(it.id, category)),
      );
    }
    this.clearPicked();
    await this.refresh();
  }

  /** Reschedules the selected items that can be rescheduled, under one undo entry. */
  async reschedulePicked(due: string) {
    const targets = this.pickedItems.filter((it) => this.canReschedule(it));
    if (!targets.length) {
      this.toast("None of those can be rescheduled here.", "info");
      return;
    }
    const before = new Map(targets.map((it) => [it.id, it.due ?? ""] as const));
    /**
     * Issues only have a date. A reminder keeps the time of day it already
     * had, since this moves the day rather than rewriting the reminder.
     */
    const to = (it: Item) => {
      if (!due) return ""; // clearing the date
      if (it.kind !== "reminder") return due.slice(0, 10); // issues carry a date only
      if (due.length > 10) return due; // the caller named a time
      const old = it.due ?? "";
      return old.length > 10 ? due + old.slice(10) : due;
    };

    const ok = await this.eachItem(targets, (it) => this.setDates(it, undefined, to(it), false));
    if (ok.length) {
      this.record(
        `reschedule ${plural(ok.length)}`,
        () => this.eachItem(ok, (it) => api.setDates(it.id, null, before.get(it.id) ?? "", it)),
        () => this.eachItem(ok, (it) => api.setDates(it.id, null, to(it), it)),
      );
    }
    this.clearPicked();
    await this.refresh();
  }

  /**
   * Deletes the selected items: a reminder goes for good, an issue is closed as
   * not planned. Nothing here can be undone, which is why the bar asks first.
   */
  async deletePicked() {
    const targets = this.pickedItems.filter((it) => canDelete(it));
    if (!targets.length) return;
    const ok = await this.eachItem(targets, (it) => api.discardItem(it.id));
    this.clearPicked();
    await this.refresh();
    if (ok.length) this.toast(`Deleted ${plural(ok.length)}.`, "info");
  }

  // ---- hand-arranged order --------------------------------------------------

  /** `items` in the group's saved order. Anything new keeps its sort place and goes last. */
  orderIn(key: string, items: Item[]): Item[] {
    const saved = this.order[key];
    if (!saved?.length) return items;
    const rank = new Map(saved.map((id, i) => [id, i] as const));
    // Sorting is stable, so items with no saved place hold their relative order.
    return [...items].sort((a, b) => (rank.get(a.id) ?? Infinity) - (rank.get(b.id) ?? Infinity));
  }

  /** Records a drag. `ids` is that group's complete new order. */
  setOrder(key: string, ids: string[]) {
    // A drag is a good moment to forget items that no longer exist, so the
    // saved order can't grow forever.
    const live = new Set(this.items.map((it) => it.id));
    const next: Record<string, string[]> = { [key]: ids.filter((id) => live.has(id)) };
    for (const [k, v] of Object.entries(this.order)) {
      if (k === key) continue;
      const kept = v.filter((id) => live.has(id));
      if (kept.length) next[k] = kept;
    }
    this.order = next;
    this.savePrefs();
  }

  inTab(item: { categories: string[] }, tab: string) {
    if (tab === "all") return true;
    if (tab === "inbox") return item.categories.length === 0;
    return item.categories.includes(tab);
  }

  matches(item: Item) {
    if (this.sourceFilter === "reminders" && item.kind !== "reminder") return false;
    if (this.sourceFilter === "github" && item.kind === "reminder") return false;
    const q = this.query.trim().toLowerCase();
    if (!q) return true;
    return q.split(/\s+/).every((term) => {
      if (term.startsWith("#") && /^#\d+$/.test(term)) return item.number === Number(term.slice(1));
      return (
        item.title.toLowerCase().includes(term) ||
        item.containerName.toLowerCase().includes(term) ||
        (item.body?.toLowerCase().includes(term) ?? false) ||
        item.labels.some((l) => l.name.toLowerCase().includes(term)) ||
        (item.status?.toLowerCase().includes(term) ?? false)
      );
    });
  }

  private matchesSource(item: Item) {
    if (this.sourceFilter === "reminders") return item.kind === "reminder";
    if (this.sourceFilter === "github") return item.kind !== "reminder";
    return true;
  }

  // ---- actions --------------------------------------------------------------

  async toggle(item: Item, recordUndo = true) {
    const completed = !item.completed;
    // The real completion time arrives with the next snapshot; until then this
    // keeps the item under today in the Completed view rather than "Earlier".
    const completedAt = completed ? new Date().toISOString() : null;
    this.overrides[item.id] = { ...this.overrides[item.id], completed, completedAt };
    if (completed) {
      this.lingering[item.id] = { ...item, completed, completedAt };
      setTimeout(() => {
        delete this.lingering[item.id];
        if (this.overrides[item.id]?.completed) delete this.overrides[item.id];
      }, LINGER_MS);
    } else {
      delete this.lingering[item.id];
    }
    try {
      await api.setCompleted(item.id, completed, item);
      if (recordUndo) {
        // `toggle` flips whatever state it's given, so pass the opposite of the target.
        const before = { ...item };
        const after = { ...item, completed };
        this.record(
          `${completed ? "complete" : "reopen"} “${item.title}”`,
          () => this.toggle(after, false),
          () => this.toggle(before, false),
        );
      }
      if (!completed) {
        // Reopened items come back through the snapshot; drop the override once it's there.
        await this.refresh();
        delete this.overrides[item.id];
      }
    } catch (e) {
      delete this.overrides[item.id];
      delete this.lingering[item.id];
      // When undoing, the caller reports the failure.
      if (!recordUndo) throw e;
      this.toast(errorText(e));
    }
  }

  async setCategory(item: Item, category: string | null, recordUndo = true) {
    const prev = item.categories;
    this.overrides[item.id] = { ...this.overrides[item.id], categories: category ? [category] : [] };
    try {
      await api.setCategory(item.id, category);
      if (recordUndo) {
        const name = this.categories.find((c) => c.id === category)?.name ?? "Inbox";
        this.record(
          `move “${item.title}” to ${name}`,
          () => api.updateItem(item.id, restorePatch(item), item),
          () => api.setCategory(item.id, category),
        );
        await this.refresh();
      }
    } catch (e) {
      this.overrides[item.id] = { ...this.overrides[item.id], categories: prev };
      // In a batch the caller counts the failures and reports them once.
      if (!recordUndo) throw e;
      this.toast(errorText(e));
    } finally {

      // Let the fresh snapshot speak for itself.
      setTimeout(() => {
        const o = this.overrides[item.id];
        if (o) {
          delete o.categories;
          if (Object.keys(o).length === 0) delete this.overrides[item.id];
        }
      }, 1500);
    }
  }

  /** Remembers how to reverse (and re-apply) a change that just succeeded. */
  record(label: string, undo: () => Promise<unknown>, redo: () => Promise<unknown>): UndoEntry {
    const entry = { label, undo, redo };
    this.undoStack.push(entry);
    if (this.undoStack.length > UNDO_LIMIT) this.undoStack.shift();
    // A new change starts a new branch of history.
    this.redoStack = [];
    return entry;
  }

  /**
   * Reverses the most recent change (⌘Z / Ctrl+Z), or a specific one, e.g.
   * from a toast's Undo button.
   */
  async undo(target?: UndoEntry) {
    let entry: UndoEntry | undefined;
    if (target) {
      const i = this.undoStack.indexOf(target);
      if (i < 0 || this.undoing) return; // already undone, or busy
      entry = this.undoStack.splice(i, 1)[0];
    } else if (!this.undoing) {
      entry = this.undoStack.pop();
      if (!entry) return this.toast("Nothing to undo", "info");
    }
    if (entry) await this.step(entry, "undo");
  }

  /** Re-applies the most recently undone change (⌘⇧Z / Ctrl+Shift+Z). */
  async redo() {
    if (this.undoing) return;
    const entry = this.redoStack.pop();
    if (!entry) return this.toast("Nothing to redo", "info");
    await this.step(entry, "redo");
  }

  /** Runs one side of an entry and moves it to the other stack if it worked. */
  private async step(entry: UndoEntry, dir: "undo" | "redo") {
    this.undoing = true;
    try {
      await entry[dir]();
      (dir === "undo" ? this.redoStack : this.undoStack).push(entry);
      await this.refresh();
      this.toast(`${dir === "undo" ? "Undid" : "Redid"} ${entry.label}`, "info");
    } catch (e) {
      this.toast(`Couldn't ${dir} ${entry.label}: ${errorText(e)}`);
    } finally {
      this.undoing = false;
    }
  }

  /** Steps the calendar by one month, week or day. */
  calShift(dir: number) {
    const d = parseYmd(this.calDate);
    const next =
      this.calView === "month" ? addMonths(d, dir) : addDays(d, dir * (this.calView === "week" ? 7 : 1));
    this.calDate = ymd(next);
  }

  calToday() {
    this.calDate = ymd(new Date());
  }

  /** Cycles List → Calendar → Gantt (V); from Completed it goes back to List. */
  nextView() {
    const order: View[] = ["list", "calendar", "gantt"];
    this.setView(order[(order.indexOf(this.view) + 1) % order.length]);
  }

  setView(view: View) {
    this.view = view;
    this.menuFor = null;
    this.clearPicked();
    this.savePrefs();
  }

  /** Whether an item's dates can be changed from the calendar or Gantt chart. */
  canReschedule(item: Item, which: "start" | "due" = "due"): boolean {
    if (item.completed) return false;
    if (item.kind === "reminder") return which === "due";
    // GitLab issues have their own due date (but no start date).
    if (item.provider === "gitlab") return which === "due" && item.kind === "issue";
    const pd = item.projectDates;
    return !!pd && !!(which === "start" ? pd.startField : pd.dueField);
  }

  /** Moves an item's due date (calendar drag). Reminders keep their time unless `due` has one. */
  async reschedule(item: Item, due: string) {
    if (due === (item.due ?? "")) return;
    if (!this.canReschedule(item)) {
      this.toast("This item's date comes from a milestone or iteration — change it on GitHub.", "info");
      return;
    }
    await this.setDates(item, undefined, item.kind === "reminder" ? due : due.slice(0, 10));
  }

  /**
   * Changes an item's start and/or due date, with undo. `undefined` leaves a
   * date alone; `""` clears it.
   */
  async setDates(item: Item, start: string | undefined, due: string | undefined, recordUndo = true) {

    const prev = { start: item.start ?? "", due: item.due ?? "" };
    const changed: Partial<Item> = {};
    if (start !== undefined) changed.start = start || null;
    if (due !== undefined) changed.due = due || null;
    this.overrides[item.id] = { ...this.overrides[item.id], ...changed };
    const arg = (v: string | undefined) => (v === undefined ? null : v);
    try {
      const updated = await api.setDates(item.id, arg(start), arg(due), item);
      if (recordUndo) {
        this.record(
          `reschedule “${item.title}”`,
          () =>
            api.setDates(
              item.id,
              start === undefined ? null : prev.start,
              due === undefined ? null : prev.due,
              updated,
            ),
          () => api.setDates(item.id, arg(start), arg(due), item),
        );
        await this.refresh();
      }
    } catch (e) {
      // In a batch the caller counts the failures and reports them once.
      if (!recordUndo) throw e;
      this.toast(errorText(e));
    } finally {

      const o = this.overrides[item.id];
      if (o) {
        delete o.start;
        delete o.due;
        if (Object.keys(o).length === 0) delete this.overrides[item.id];
      }
    }
  }

  editEvent(e: CalendarEvent) {
    this.menuFor = null;
    this.selectedId = e.id;
    this.editingEvent = e;
  }

  openEvent(e: CalendarEvent) {
    api.openEvent(e.id).catch((err) => this.toast(errorText(err)));
  }

  /** Edits an event (drag, resize, or the editor), with undo. Returns false on failure. */
  async updateEvent(e: CalendarEvent, patch: EventPatch, label = `edit “${e.title}”`): Promise<boolean> {
    // The inverse restores each changed field to its old value.
    const inverse: EventPatch = {};
    if (patch.title !== undefined) inverse.title = e.title;
    if (patch.notes !== undefined) inverse.notes = e.notes ?? "";
    if (patch.location !== undefined) inverse.location = e.location ?? "";
    if (patch.calendarId !== undefined) inverse.calendarId = e.calendarId;
    if (patch.start !== undefined || patch.end !== undefined || patch.allDay !== undefined) {
      inverse.start = e.start;
      inverse.end = e.end;
      inverse.allDay = e.allDay;
    }
    const optimistic: Partial<CalendarEvent> = {};
    if (patch.start !== undefined) optimistic.start = patch.start;
    if (patch.end !== undefined) optimistic.end = patch.end;
    if (patch.allDay !== undefined) optimistic.allDay = patch.allDay;
    if (patch.title !== undefined) optimistic.title = patch.title;
    this.eventOverrides[e.id] = optimistic;
    try {
      const updated = await api.updateEvent(e.id, patch, e);
      this.record(
        label,
        () => api.updateEvent(updated.id, inverse, updated),
        () => api.updateEvent(e.id, patch, e),
      );
      await this.refresh();
      return true;
    } catch (err) {
      this.toast(errorText(err));
      return false;
    } finally {
      delete this.eventOverrides[e.id];
    }
  }

  edit(item: Item) {
    this.menuFor = null;
    this.selectedId = item.id;
    this.editing = item;
  }

  open(item: Item) {
    api.openItem(item.id).catch((e) => this.toast(errorText(e)));
  }

  openSettings(section = "accounts") {
    this.settingsSection = section;
    this.settingsOpen = true;
  }

  toast(text: string, kind: Toast["kind"] = "error", action?: Toast["action"]) {
    const id = ++this.toastSeq;
    this.toasts.push({ id, text, kind, action });
    // Toasts with a button (like Undo) stay long enough to reach for it.
    setTimeout(() => this.dismiss(id), action ? 8000 : kind === "error" ? 7000 : 3500);
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }
}

/** 0 means "no priority", which sorts after every real one. */
const prio = (i: Item) => i.priority || 10;

export const SORT_NAMES: Record<SortBy, string> = {
  due: "Due date",
  priority: "Priority",
  title: "Title",
  updated: "Last updated",
  container: "List / repo",
  manual: "Manual",
};

/** Each sort falls back to the due-date order so equal keys stay stable. */
const SORTS: Record<SortBy, (a: Item, b: Item) => number> = {
  due: (a, b) => compareItems(a, b),
  priority: (a, b) => prio(a) - prio(b) || compareItems(a, b),
  title: (a, b) => a.title.localeCompare(b.title) || compareItems(a, b),
  updated: (a, b) => (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "") || compareItems(a, b),
  container: (a, b) => a.containerName.localeCompare(b.containerName) || compareItems(a, b),
  // The hand-arranged order is per group, so it is applied where the groups are
  // built (`store.orderIn`). Anything with no saved place falls back to due date.
  manual: (a, b) => compareItems(a, b),
};

/** `n items`, or `1 item`. */
function plural(n: number) {
  return `${n} item${n === 1 ? "" : "s"}`;
}

/**
 * Whether `discard_item` can take this: it removes a reminder and closes an
 * issue as not planned, but has nothing to do for a draft or a pull request.
 */
export function canDelete(item: Item) {
  return item.kind === "reminder" || item.kind === "issue";
}

/**
 * Undoes a move between areas. Moving rewrites GitHub labels, or a reminder's
 * list, title and notes, so putting it back means restoring what they were.
 */
function restorePatch(item: Item): ItemPatch {
  return item.kind === "reminder"
    ? { listId: item.containerId, title: item.title, body: item.body ?? "" }
    : { labels: item.labels.map((l) => l.name) };
}

/** Due date first (soonest, undated last), then priority, then most recently updated. */

function compareItems(a: Item, b: Item) {
  if (a.due !== b.due) {
    if (!a.due) return 1;
    if (!b.due) return -1;
    return a.due < b.due ? -1 : 1;
  }
  if (prio(a) !== prio(b)) return prio(a) - prio(b);
  return (b.updatedAt ?? "").localeCompare(a.updatedAt ?? "");
}

export const store = new Store();

// ---- date helpers -------------------------------------------------------------

export function todayStr(offsetDays = 0) {
  const d = new Date();
  d.setDate(d.getDate() + offsetDays);
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
}

export type DueBucket = "overdue" | "today" | "tomorrow" | "week" | "later" | "none";

export function dueBucket(due: string | null): DueBucket {
  if (!due) return "none";
  const day = due.slice(0, 10);
  const today = todayStr();
  if (day < today) return "overdue";
  if (day === today) {
    if (due.length > 10 && new Date(due) < new Date()) return "overdue";
    return "today";
  }
  if (day === todayStr(1)) return "tomorrow";
  if (day <= todayStr(7)) return "week";
  return "later";
}

export function formatDue(due: string): string {
  const bucket = dueBucket(due);
  const time =
    due.length > 10 ? new Date(due).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" }) : "";
  const [y, m, d] = due.slice(0, 10).split("-").map(Number);
  const date = new Date(y, m - 1, d);
  let label: string;
  if (bucket === "today" || (bucket === "overdue" && due.slice(0, 10) === todayStr())) label = "Today";
  else if (bucket === "tomorrow") label = "Tomorrow";
  else if (due.slice(0, 10) === todayStr(-1)) label = "Yesterday";
  else if (bucket === "week") label = date.toLocaleDateString(undefined, { weekday: "long" });
  else
    label = date.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      year: y === new Date().getFullYear() ? undefined : "numeric",
    });
  return time ? `${label}, ${time}` : label;
}

export function relativeTime(iso: string | null): string {
  if (!iso) return "never";
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}
