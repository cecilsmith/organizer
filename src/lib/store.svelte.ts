import { listen } from "@tauri-apps/api/event";
import { api, errorText, type Item, type ItemPatch, type Snapshot } from "./api";
import { addDays, addMonths, parseYmd, ymd } from "./dates";

export type SourceFilter = "all" | "reminders" | "github";
export type GroupBy = "source" | "due";
/** Whether reminder notes show under every row, or only the selected one. */
export type ShowNotes = "always" | "selected";
export type View = "list" | "calendar";
export type CalView = "month" | "week" | "day";

/** One undoable change: how to reverse it, and how to apply it again. */
export interface UndoEntry {
  /** Describes the original change, e.g. `complete “Buy milk”`. */
  label: string;
  undo: () => Promise<unknown>;
  redo: () => Promise<unknown>;
}

const UNDO_LIMIT = 50;

export interface Toast {
  id: number;
  text: string;
  kind: "error" | "info";
  action?: { label: string; run: () => void };
}

/** Completed items stay on screen this long so a mis-click can be undone. */
const LINGER_MS = 4000;
const FOCUS_SYNC_AFTER_MS = 30_000;

const PREFS_KEY = "organizer.prefs";

interface Prefs {
  tab: string;
  groupBy: GroupBy;
  sourceFilter: SourceFilter;
  showNotes: ShowNotes;
  view: View;
  calView: CalView;
  weekStart: number;
  showUnscheduled: boolean;
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
  tab = $state<string>("all");
  query = $state("");
  sourceFilter = $state<SourceFilter>("all");
  groupBy = $state<GroupBy>("source");
  showNotes = $state<ShowNotes>("selected");
  view = $state<View>("list");
  calView = $state<CalView>("month");
  /** The day the calendar is showing (any day within the visible month/week). */
  calDate = $state<string>(ymd(new Date()));
  /** 0 = Sunday, 1 = Monday. */
  weekStart = $state(0);
  showUnscheduled = $state(true);
  /** Set to prefill the quick-add due date (`YYYY-MM-DD` or with `THH:MM`). */
  quickAddDue = $state<string | null>(null);
  settingsOpen = $state(false);
  settingsSection = $state<string>("accounts");
  toasts = $state<Toast[]>([]);
  /** Item whose "move to area" menu is open. */
  menuFor = $state<string | null>(null);
  selectedId = $state<string | null>(null);
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
    if (p.tab) this.tab = p.tab;
    if (p.groupBy) this.groupBy = p.groupBy;
    if (p.showNotes) this.showNotes = p.showNotes;
    if (p.view) this.view = p.view;
    if (p.calView) this.calView = p.calView;
    if (p.weekStart === 0 || p.weekStart === 1) this.weekStart = p.weekStart;
    if (typeof p.showUnscheduled === "boolean") this.showUnscheduled = p.showUnscheduled;
    if (p.sourceFilter) this.sourceFilter = p.sourceFilter;
  }

  savePrefs() {
    try {
      localStorage.setItem(
        PREFS_KEY,
        JSON.stringify({
          tab: this.tab,
          groupBy: this.groupBy,
          sourceFilter: this.sourceFilter,
          showNotes: this.showNotes,
          view: this.view,
          calView: this.calView,
          weekStart: this.weekStart,
          showUnscheduled: this.showUnscheduled,
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

  /** Open-item counts per tab id ("all", "inbox", or a category id). */
  counts: Record<string, number> = $derived.by(() => {
    const c: Record<string, number> = { all: 0, inbox: 0 };
    for (const it of this.items) {
      if (it.completed || !this.matchesSource(it)) continue;
      c.all++;
      if (it.categories.length === 0) c.inbox++;
      for (const cat of it.categories) c[cat] = (c[cat] ?? 0) + 1;
    }
    return c;
  });

  visible: Item[] = $derived.by(() =>
    this.items.filter((it) => this.inTab(it, this.tab) && this.matches(it)).sort(compareItems),
  );

  inTab(item: Item, tab: string) {
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
    this.overrides[item.id] = { ...this.overrides[item.id], completed };
    if (completed) {
      this.lingering[item.id] = { ...item, completed };
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

  async setCategory(item: Item, category: string | null) {
    const prev = item.categories;
    this.overrides[item.id] = { ...this.overrides[item.id], categories: category ? [category] : [] };
    try {
      await api.setCategory(item.id, category);
      // Moving an area rewrites labels (GitHub) or list, notes and title (Reminders); undo restores them.
      const restore: ItemPatch =
        item.kind === "reminder"
          ? { listId: item.containerId, title: item.title, body: item.body ?? "" }
          : { labels: item.labels.map((l) => l.name) };
      const name = this.categories.find((c) => c.id === category)?.name ?? "Inbox";
      this.record(
        `move “${item.title}” to ${name}`,
        () => api.updateItem(item.id, restore, item),
        () => api.setCategory(item.id, category),
      );
      await this.refresh();
    } catch (e) {
      this.overrides[item.id] = { ...this.overrides[item.id], categories: prev };
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

  setView(view: View) {
    this.view = view;
    this.menuFor = null;
    this.savePrefs();
  }

  /**
   * Moves a reminder to another day (keeping its time unless `due` has one).
   * GitHub dates come from milestones and project fields, so they can't be
   * dragged here.
   */
  async reschedule(item: Item, due: string) {
    if (item.kind !== "reminder") {
      this.toast("GitHub dates come from milestones or project fields — change them on GitHub.", "info");
      return;
    }
    if (due === item.due) return;
    const prev = item.due ?? "";
    this.overrides[item.id] = { ...this.overrides[item.id], due };
    try {
      const updated = await api.updateItem(item.id, { due }, item);
      this.record(
        `reschedule “${item.title}”`,
        () => api.updateItem(item.id, { due: prev }, updated),
        () => api.updateItem(item.id, { due }, item),
      );
      await this.refresh();
    } catch (e) {
      this.toast(errorText(e));
    } finally {
      const o = this.overrides[item.id];
      if (o) {
        delete o.due;
        if (Object.keys(o).length === 0) delete this.overrides[item.id];
      }
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

/** Due date first (soonest, undated last), then priority, then most recently updated. */
function compareItems(a: Item, b: Item) {
  if (a.due !== b.due) {
    if (!a.due) return 1;
    if (!b.due) return -1;
    return a.due < b.due ? -1 : 1;
  }
  const pa = a.priority || 10;
  const pb = b.priority || 10;
  if (pa !== pb) return pa - pb;
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
