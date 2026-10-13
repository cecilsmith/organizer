<script lang="ts">
  import { onMount, tick } from "svelte";
  import type { Item } from "$lib/api";
  import { addDays, parseYmd, ymd } from "$lib/dates";
  import { store, dueBucket, type GanttGroup, type GanttZoom } from "$lib/store.svelte";
  import Icon from "../Icon.svelte";
  import { sourceIcon } from "$lib/providers";

  const LABEL_W = 260;
  const ROW_H = 30;
  const DAY_W: Record<GanttZoom, number> = { day: 36, week: 14, month: 4 };
  const DAY_MS = 86_400_000;

  const dayW = $derived(DAY_W[store.ganttZoom]);
  const today = ymd(new Date());

  // ---- rows ---------------------------------------------------------------------

  interface Span {
    item: Item;
    /** First and last day (inclusive). */
    start: string;
    end: string;
    /** A due date with no start: drawn as a diamond. */
    milestone: boolean;
    depth: number;
  }

  interface Group {
    key: string;
    title: string;
    color: string | null;
    icon: string;
    rows: Span[];
  }

  function spanOf(item: Item, depth = 0): Span | null {
    const s = item.start?.slice(0, 10) ?? null;
    const d = item.due?.slice(0, 10) ?? null;
    if (!s && !d) return null;
    if (s && d) return { item, start: s <= d ? s : d, end: s <= d ? d : s, milestone: false, depth };
    if (d) return { item, start: d, end: d, milestone: true, depth };
    return { item, start: s!, end: s!, milestone: false, depth };
  }

  const dated = $derived(store.visible.filter((i) => i.start || i.due));
  const undatedCount = $derived(store.visible.length - dated.length);

  const projectTitles = $derived(new Map(store.snap?.projects.map((p) => [p.id, p.title]) ?? []));

  const groups: Group[] = $derived.by(() => {
    const map = new Map<string, Group>();
    const add = (key: string, init: () => Omit<Group, "rows" | "key">, item: Item) => {
      let g = map.get(key);
      if (!g) map.set(key, (g = { key, rows: [], ...init() }));
      return g;
    };
    const members = new Map<string, Item[]>();
    for (const it of dated) {
      let g: Group;
      if (store.ganttGroup === "area") {
        const cat = store.categories.find((c) => it.categories.includes(c.id));
        g = add(cat?.id ?? "~inbox", () => ({ title: cat?.name ?? "Inbox", color: cat?.color ?? null, icon: "inbox" }), it);
      } else if (it.kind === "reminder") {
        const list = store.snap?.reminderLists.find((l) => l.id === it.containerId);
        g = add(`rem:${it.containerId}`, () => ({ title: it.containerName, color: list?.color ?? null, icon: "list" }), it);
      } else {
        // Project work groups by project — that's what a roadmap is organized around.
        const pid = it.projectIds[0] ?? (it.kind === "draftIssue" ? it.containerId : undefined);
        const title = pid ? (projectTitles.get(pid) ?? it.containerName) : it.containerName;
        g = add(pid ? `proj:${pid}` : `repo:${it.provider}:${it.containerName}`, () => ({ title, color: null, icon: pid ? "stack" : sourceIcon(it) }), it);
      }
      const list = members.get(g.key) ?? [];
      list.push(it);
      members.set(g.key, list);
    }
    for (const [key, items] of members) map.get(key)!.rows = nest(items);
    return [...map.values()].sort((a, b) => {
      const order = (k: string) => (k.startsWith("proj:") ? 0 : k.startsWith("repo:") ? 1 : 2);
      return order(a.key) - order(b.key) || a.title.localeCompare(b.title);
    });
  });

  /** Earliest first; sub-issues go right under their parent. */
  function nest(items: Item[]): Span[] {
    const here = new Set(items.map((i) => i.id));
    const kids = new Map<string, Item[]>();
    const roots: Item[] = [];
    for (const it of items) {
      if (it.parentId && here.has(it.parentId)) kids.set(it.parentId, [...(kids.get(it.parentId) ?? []), it]);
      else roots.push(it);
    }
    const key = (i: Item) => (i.start ?? i.due ?? "").slice(0, 10);
    const sort = (list: Item[]) => list.sort((a, b) => key(a).localeCompare(key(b)) || a.title.localeCompare(b.title));
    const out: Span[] = [];
    const seen = new Set<string>();
    const visit = (it: Item, depth: number) => {
      if (seen.has(it.id)) return;
      seen.add(it.id);
      const s = spanOf(it, depth);
      if (s) out.push(s);
      for (const c of sort(kids.get(it.id) ?? [])) visit(c, depth + 1);
    };
    for (const r of sort(roots)) visit(r, 0);
    return out;
  }

  // ---- timeline range -------------------------------------------------------------

  /** Visible range: everything scheduled, plus room around it and around today. */
  const range = $derived.by(() => {
    let min = addDays(parseYmd(today), -14);
    let max = addDays(parseYmd(today), store.ganttZoom === "month" ? 365 : store.ganttZoom === "week" ? 120 : 45);
    for (const g of groups)
      for (const r of g.rows) {
        const s = parseYmd(r.start);
        const e = parseYmd(r.end);
        if (s < min) min = addDays(s, -7);
        if (e > max) max = addDays(e, 14);
      }
    // Start on the first of a month so the top scale reads cleanly.
    min = new Date(min.getFullYear(), min.getMonth(), 1);
    const days = Math.round((max.getTime() - min.getTime()) / DAY_MS) + 1;
    return { start: min, days };
  });

  function dayIndex(date: string) {
    return Math.round((parseYmd(date).getTime() - range.start.getTime()) / DAY_MS);
  }
  const x = (date: string) => dayIndex(date) * dayW;

  /** Two scale rows: months (or years) on top, days/weeks/months below. */
  const scale = $derived.by(() => {
    const top: { label: string; left: number; width: number }[] = [];
    const bottom: { label: string; left: number; width: number; weekend?: boolean; today?: boolean }[] = [];
    const days = Array.from({ length: range.days }, (_, i) => addDays(range.start, i));
    const zoom = store.ganttZoom;
    const topKey = (d: Date) => (zoom === "month" ? `${d.getFullYear()}` : `${d.getFullYear()}-${d.getMonth()}`);
    days.forEach((d, i) => {
      const k = topKey(d);
      const last = top.at(-1);
      if (last && (last as any).k === k) last.width += dayW;
      else
        top.push(
          Object.assign(
            {
              label:
                zoom === "month"
                  ? `${d.getFullYear()}`
                  : d.toLocaleDateString(undefined, { month: "long", year: d.getMonth() === 0 || i === 0 ? "numeric" : undefined }),
              left: i * dayW,
              width: dayW,
            },
            { k },
          ),
        );
      const startsUnit =
        zoom === "day" ? true : zoom === "week" ? d.getDay() === store.weekStart || i === 0 : d.getDate() === 1 || i === 0;
      if (startsUnit) {
        bottom.push({
          label:
            zoom === "day"
              ? `${d.getDate()}`
              : zoom === "week"
                ? d.toLocaleDateString(undefined, { month: "short", day: "numeric" })
                : d.toLocaleDateString(undefined, { month: "short" }),
          left: i * dayW,
          width: dayW,
          weekend: zoom === "day" && (d.getDay() === 0 || d.getDay() === 6),
          today: zoom === "day" && ymd(d) === today,
        });
      } else {
        bottom.at(-1)!.width += dayW;
      }
    });
    return { top, bottom };
  });

  const width = $derived(range.days * dayW);

  // ---- scrolling ---------------------------------------------------------------------

  let scroller = $state<HTMLDivElement>();
  let scrollX = $state(0);

  /** Month (or year) at the left edge of the timeline, shown in the corner. */
  const cornerLabel = $derived.by(() => {
    const d = addDays(range.start, Math.max(0, Math.floor(scrollX / dayW)));
    return store.ganttZoom === "month"
      ? `${d.getFullYear()}`
      : d.toLocaleDateString(undefined, { month: "long", year: "numeric" });
  });

  export async function scrollToToday(smooth = true) {
    await tick();
    if (!scroller) return;
    // The label column covers the first LABEL_W pixels, so place today a third
    // of the way across the timeline area that's actually visible.
    const target = x(today) - (scroller.clientWidth - LABEL_W) / 3;
    scroller.scrollTo({ left: Math.max(0, target), behavior: smooth ? "smooth" : "auto" });
  }

  /** The date at the center of the visible timeline, so zooming keeps your place. */
  function centerDate(): string | null {
    if (!scroller) return null;
    const px = scroller.scrollLeft + (scroller.clientWidth - LABEL_W) / 2;
    return ymd(addDays(range.start, Math.floor(px / dayW)));
  }

  onMount(() => {
    scrollToToday(false);
  });

  async function zoomTo(z: GanttZoom) {
    const center = centerDate();
    store.ganttZoom = z;
    store.savePrefs();
    await tick();
    if (scroller && center) scroller.scrollLeft = Math.max(0, x(center) - (scroller.clientWidth - LABEL_W) / 2);
  }

  // ---- dragging --------------------------------------------------------------------

  type Mode = "move" | "start" | "end";
  let drag = $state<{ span: Span; mode: Mode; originX: number; delta: number; moved: boolean } | null>(null);

  function canMove(span: Span) {
    const it = span.item;
    if (span.milestone) return store.canReschedule(it, "due");
    // Moving a bar shifts both ends, so both must be editable.
    return store.canReschedule(it, "start") && store.canReschedule(it, "due");
  }

  function onpointerdown(e: PointerEvent, span: Span, mode: Mode) {
    if (e.button !== 0) return;
    e.stopPropagation();
    store.selectedId = span.item.id;
    const allowed = mode === "move" ? canMove(span) : store.canReschedule(span.item, mode === "start" ? "start" : "due");
    if (!allowed) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    drag = { span, mode, originX: e.clientX, delta: 0, moved: false };
  }

  function onpointermove(e: PointerEvent) {
    if (!drag) return;
    const delta = Math.round((e.clientX - drag.originX) / dayW);
    if (delta !== drag.delta) drag = { ...drag, delta, moved: true };
  }

  function onpointerup() {
    if (!drag) return;
    const { span, mode, delta } = drag;
    drag = null;
    if (!delta) return;
    const shift = (d: string) => ymd(addDays(parseYmd(d), delta));
    const it = span.item;
    // Reminders keep their time of day.
    const dueWithTime = (day: string) => (it.kind === "reminder" && it.due && it.due.length > 10 ? `${day}${it.due.slice(10)}` : day);
    if (span.milestone) {
      store.setDates(it, undefined, dueWithTime(shift(span.end)));
    } else if (mode === "move") {
      store.setDates(it, shift(span.start), shift(span.end));
    } else if (mode === "start") {
      const start = shift(span.start);
      if (start <= span.end) store.setDates(it, start, undefined);
    } else {
      const end = shift(span.end);
      if (end >= span.start) store.setDates(it, undefined, end);
    }
  }

  /** The span as currently drawn, including an in-progress drag. */
  function live(span: Span): { start: string; end: string } {
    if (!drag || drag.span.item.id !== span.item.id || !drag.delta) return span;
    const shift = (d: string) => ymd(addDays(parseYmd(d), drag!.delta));
    if (span.milestone || drag.mode === "move") return { start: shift(span.start), end: shift(span.end) };
    if (drag.mode === "start") return { start: [shift(span.start), span.end].sort()[0], end: span.end };
    return { start: span.start, end: [span.start, shift(span.end)].sort()[1] };
  }

  function colorOf(item: Item) {
    return store.categories.find((c) => item.categories.includes(c.id))?.color ?? "var(--text-3)";
  }

  function fmt(d: string) {
    return parseYmd(d).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  }

  function setGroup(g: GanttGroup) {
    store.ganttGroup = g;
    store.savePrefs();
  }
</script>

<svelte:window {onpointermove} {onpointerup} />

<div class="gantt">
  <div class="toolbar">
    <button class="btn" onclick={() => scrollToToday()} title="Today (T)">Today</button>
    <span class="faint note">
      {#if undatedCount}{undatedCount} item{undatedCount === 1 ? "" : "s"} without dates not shown{/if}
    </span>
    <div class="seg" role="group" aria-label="Group rows by">
      <button class:on={store.ganttGroup === "source"} onclick={() => setGroup("source")}>By project</button>
      <button class:on={store.ganttGroup === "area"} onclick={() => setGroup("area")}>By area</button>
    </div>
    <div class="seg" role="group" aria-label="Zoom">
      {#each [["day", "Days"], ["week", "Weeks"], ["month", "Months"]] as [z, label] (z)}
        <button class:on={store.ganttZoom === z} onclick={() => zoomTo(z as GanttZoom)}>{label}</button>
      {/each}
    </div>
  </div>

  {#if !groups.length}
    <div class="empty">
      <Icon name="calendar" size={28} class="faint" />
      <p class="muted">Nothing here has dates yet.</p>
      <p class="faint small">
        Bars come from GitHub Project date fields (like “Start date” and “Target date”) or iterations. Reminders and
        milestones show as ◆ on their due date.
      </p>
    </div>
  {:else}
    <div class="scroller" bind:this={scroller} class:dragging={!!drag} onscroll={(e) => (scrollX = e.currentTarget.scrollLeft)}>
      <div class="canvas" style:width="{LABEL_W + width}px" style:--label="{LABEL_W}px" style:--row="{ROW_H}px">
        <!-- Header -->
        <div class="head">
          <div class="corner">{cornerLabel}</div>
          <div class="scale" style:width="{width}px">
            <div class="tier top">
              {#each scale.top as t (t.left)}
                <div class="tick" style:left="{t.left}px" style:width="{t.width}px"><span>{t.label}</span></div>
              {/each}
            </div>
            <div class="tier bottom">
              {#each scale.bottom as t (t.left)}
                <div class="tick" class:weekend={t.weekend} class:today={t.today} style:left="{t.left}px" style:width="{t.width}px">
                  {t.label}
                </div>
              {/each}
            </div>
          </div>
        </div>

        <!-- Body -->
        <div class="body">
          <div class="grid" style:left="{LABEL_W}px" style:width="{width}px">
            {#each scale.bottom as t (t.left)}
              <div class="line" class:weekend={t.weekend} style:left="{t.left}px" style:width="{t.width}px"></div>
            {/each}
            <div class="today-line" style:left="{x(today) + dayW / 2}px"></div>
          </div>

          {#each groups as g (g.key)}
            <div class="group">
              <div class="label group-label">
                {#if g.color}<span class="dot" style:background={g.color}></span>{:else}<Icon name={g.icon} size={13} />{/if}
                <span class="name">{g.title}</span>
                <span class="faint">{g.rows.length}</span>
              </div>
            </div>
            {#each g.rows as span (span.item.id)}
              {@const it = span.item}
              {@const pos = live(span)}
              {@const color = colorOf(it)}
              {@const overdue = !it.completed && dueBucket(it.due) === "overdue"}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div
                class="row"
                class:selected={store.selectedId === it.id}
                onclick={() => (store.selectedId = it.id)}
                ondblclick={() => store.edit(it)}
              >
                <div class="label" style:padding-left="{14 + span.depth * 16}px" title={it.title}>
                  {#if span.depth}<span class="faint">↳</span>{/if}
                  <span class="name" class:done={it.completed}>{it.title}</span>
                  {#if it.number}<span class="faint">#{it.number}</span>{/if}
                </div>
                <div class="track">
                  {#if span.milestone}
                    <div
                      class="milestone"
                      class:movable={canMove(span)}
                      class:overdue
                      style:left="{x(pos.start) + dayW / 2}px"
                      style:--c={color}
                      title="{it.title} — due {fmt(pos.end)}{canMove(span) ? '' : ' (set on GitHub)'}"
                      onpointerdown={(e) => onpointerdown(e, span, "move")}
                    ></div>
                    <span class="after" style:left="{x(pos.start) + dayW / 2 + 12}px">{it.title}</span>
                  {:else}
                    {@const w = Math.max(dayW, (dayIndex(pos.end) - dayIndex(pos.start) + 1) * dayW)}
                    <div
                      class="bar"
                      class:movable={canMove(span)}
                      class:overdue
                      class:done={it.completed}
                      style:left="{x(pos.start)}px"
                      style:width="{w}px"
                      style:--c={color}
                      title="{it.title}: {fmt(pos.start)} – {fmt(pos.end)}{canMove(span) ? '' : ' (dates come from an iteration or are set on GitHub)'}"
                      onpointerdown={(e) => onpointerdown(e, span, "move")}
                    >
                      {#if store.canReschedule(it, "start")}
                        <span class="handle start" onpointerdown={(e) => onpointerdown(e, span, "start")}></span>
                      {/if}
                      <span class="bar-label">{w > 60 ? it.title : ""}</span>
                      {#if store.canReschedule(it, "due")}
                        <span class="handle end" onpointerdown={(e) => onpointerdown(e, span, "end")}></span>
                      {/if}
                    </div>
                    {#if w <= 60}<span class="after" style:left="{x(pos.start) + w + 6}px">{it.title}</span>{/if}
                  {/if}
                  {#if drag && drag.span.item.id === it.id && drag.delta}
                    <span class="drag-dates" style:left="{x(pos.start)}px">{fmt(pos.start)}{pos.start !== pos.end ? ` – ${fmt(pos.end)}` : ""}</span>
                  {/if}
                </div>
              </div>
            {/each}
          {/each}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .gantt {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 0 20px 14px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-bottom: 10px;
  }
  .note {
    flex: 1;
    font-size: 12px;
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: 7px;
    background: var(--bg-input);
  }
  .seg button {
    height: 24px;
    padding: 0 11px;
    border-radius: 5px;
    font-size: 12px;
    color: var(--text-2);
  }
  .seg button.on {
    background: var(--bg-elev);
    color: var(--text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
    font-weight: 500;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: 90px;
    text-align: center;
  }
  .empty p {
    margin: 8px 0 0;
    max-width: 420px;
  }
  .small {
    font-size: 12px;
  }

  .scroller {
    flex: 1;
    min-height: 0;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 8px;
  }
  .scroller.dragging {
    cursor: grabbing;
    user-select: none;
  }
  .canvas {
    position: relative;
    min-height: 100%;
  }

  /* Header: sticky on top; its corner also sticks left. */
  .head {
    position: sticky;
    top: 0;
    z-index: 5;
    display: flex;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
  }
  .corner {
    position: sticky;
    left: 0;
    z-index: 6;
    width: var(--label);
    flex: none;
    display: flex;
    align-items: flex-end;
    padding: 0 0 6px 14px;
    background: var(--bg);
    border-right: 1px solid var(--border);
    font-size: 13px;
    font-weight: 600;
  }
  .scale {
    position: relative;
    height: 44px;
  }
  .tier {
    position: absolute;
    left: 0;
    right: 0;
    height: 22px;
  }
  .tier.top {
    top: 0;
  }
  .tier.bottom {
    top: 22px;
  }
  .tick {
    position: absolute;
    top: 0;
    height: 22px;
    padding: 0 5px;
    font-size: 11px;
    line-height: 22px;
    white-space: nowrap;
    overflow: hidden;
    border-left: 1px solid var(--border);
    color: var(--text-2);
  }
  .tier.top .tick {
    font-weight: 600;
    color: var(--text);
  }
  .tier.bottom .tick {
    text-align: left;
    font-variant-numeric: tabular-nums;
  }
  .tick.weekend {
    color: var(--text-3);
  }
  .tick.today {
    color: white;
    background: var(--danger);
    font-weight: 600;
    border-radius: 4px;
  }

  .body {
    position: relative;
  }
  .grid {
    position: absolute;
    top: 0;
    bottom: 0;
    pointer-events: none;
  }
  .line {
    position: absolute;
    top: 0;
    bottom: 0;
    border-left: 1px solid var(--border);
  }
  .line.weekend {
    background: color-mix(in srgb, var(--bg-input) 70%, transparent);
  }
  .today-line {
    position: absolute;
    top: 0;
    bottom: 0;
    border-left: 2px solid var(--danger);
    /* Behind the bars, so it never cuts through their labels. */
    z-index: 0;
  }

  .group,
  .row {
    position: relative;
    display: flex;
    height: var(--row);
  }
  .group {
    height: 28px;
    margin-top: 4px;
  }
  .label {
    position: sticky;
    left: 0;
    z-index: 3;
    width: var(--label);
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px 0 14px;
    background: var(--bg);
    border-right: 1px solid var(--border);
    font-size: 12.5px;
    min-width: 0;
  }
  .group-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
    padding-left: 10px;
  }
  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name.done {
    text-decoration: line-through;
    color: var(--text-3);
  }
  .row:hover .label,
  .row:hover .track {
    background-color: var(--bg-hover);
  }
  /* The label column is sticky, so it needs an opaque background. */
  .row.selected .label {
    background-color: color-mix(in srgb, var(--accent) 12%, var(--bg));
  }
  .row.selected .track {
    background-color: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .track {
    position: relative;
    flex: 1;
  }

  .bar {
    position: absolute;
    top: 6px;
    height: calc(var(--row) - 12px);
    display: flex;
    align-items: center;
    border-radius: 5px;
    /* Light mode: soft tint with dark, area-colored text. */
    background: color-mix(in srgb, var(--c) 26%, var(--bg));
    border: 1px solid var(--c);
    color: color-mix(in srgb, var(--c) 45%, var(--text));
    font-size: 11.5px;
    font-weight: 500;
    overflow: hidden;
    z-index: 1;
  }
  @media (prefers-color-scheme: dark) {
    .bar {
      background: color-mix(in srgb, var(--c) 70%, var(--bg));
      color: white;
    }
  }
  .bar.movable {
    cursor: grab;
  }
  .bar.overdue {
    box-shadow: 0 0 0 1.5px var(--danger);
  }
  .bar.done {
    opacity: 0.5;
  }
  .bar-label {
    flex: 1;
    min-width: 0;
    padding: 0 8px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    pointer-events: none;
  }
  .handle {
    width: 7px;
    align-self: stretch;
    flex: none;
    cursor: ew-resize;
  }
  .handle:hover {
    background: color-mix(in srgb, var(--c) 45%, transparent);
  }
  .milestone {
    position: absolute;
    top: 50%;
    width: 12px;
    height: 12px;
    margin: -6px 0 0 -6px;
    transform: rotate(45deg);
    border-radius: 2px;
    background: var(--c);
    z-index: 1;
  }
  .milestone.movable {
    cursor: grab;
  }
  .milestone.overdue {
    box-shadow: 0 0 0 2px var(--danger);
  }
  .after {
    position: absolute;
    top: 0;
    line-height: var(--row);
    font-size: 11.5px;
    color: var(--text-2);
    white-space: nowrap;
    pointer-events: none;
  }
  .drag-dates {
    position: absolute;
    top: -2px;
    transform: translateY(-100%);
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--text);
    color: var(--bg);
    font-size: 11px;
    white-space: nowrap;
    z-index: 4;
    pointer-events: none;
  }
</style>
