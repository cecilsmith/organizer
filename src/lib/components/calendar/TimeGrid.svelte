<script lang="ts">
  import { onMount } from "svelte";
  import type { Item } from "$lib/api";
  import { dueMinutes, formatHour, withTime, ymd } from "$lib/dates";
  import { store } from "$lib/store.svelte";
  import CalendarChip from "./CalendarChip.svelte";

  let { days, byDay }: { days: Date[]; byDay: Map<string, Item[]> } = $props();

  const HOUR = 56; // px per hour: 15 minutes ≈ one line of text
  /** Reminders have no duration; draw each as a half-hour block. */
  const SPAN = 30;
  const SNAP = 15; // minutes

  let scroller = $state<HTMLDivElement>();
  let now = $state(new Date());
  let drop = $state<{ day: string; minutes: number | null } | null>(null);

  const today = $derived(ymd(now));
  const nowMinutes = $derived(now.getHours() * 60 + now.getMinutes());

  /** Horizontal indent for each overlapping, later-starting item. */
  const INDENT = 14;

  interface Placed {
    item: Item;
    minutes: number;
    /** How many earlier, still-running items this one is drawn on top of. */
    indent: number;
    /** Position among items starting at the same minute, which share the width. */
    sub: number;
    subs: number;
  }

  /**
   * Like Apple Calendar: an item that starts while another is still running is
   * indented and drawn over it, so the earlier title stays readable. Items that
   * start at the same minute split the width instead.
   */
  function layout(items: Item[]): Placed[] {
    const timed = items
      .map((item) => ({ item, minutes: dueMinutes(item.due!) }))
      .filter((t): t is { item: Item; minutes: number } => t.minutes !== null)
      .sort((a, b) => a.minutes - b.minutes);
    const out: Placed[] = [];
    const ends: number[] = []; // end times of the start-groups drawn so far
    for (let i = 0; i < timed.length; ) {
      const start = timed[i].minutes;
      const group = [];
      while (i < timed.length && timed[i].minutes === start) group.push(timed[i++]);
      const indent = ends.filter((end) => end > start).length;
      ends.push(start + SPAN);
      group.forEach((t, sub) => out.push({ ...t, indent, sub, subs: group.length }));
    }
    return out;
  }

  const columns = $derived(
    days.map((d) => {
      const day = ymd(d);
      const items = byDay.get(day) ?? [];
      return {
        day,
        date: d,
        allDay: items.filter((i) => dueMinutes(i.due!) === null),
        timed: layout(items),
      };
    }),
  );

  function minutesAt(e: MouseEvent | DragEvent, col: HTMLElement) {
    const y = e.clientY - col.getBoundingClientRect().top;
    const m = Math.floor((y / HOUR) * 60 / SNAP) * SNAP;
    return Math.max(0, Math.min(24 * 60 - SNAP, m));
  }

  function itemFrom(e: DragEvent) {
    const id = e.dataTransfer?.getData("text/organizer-item");
    return store.items.find((i) => i.id === id);
  }

  function dropOnSlot(e: DragEvent, day: string) {
    e.preventDefault();
    const item = itemFrom(e);
    const minutes = drop?.minutes ?? null;
    drop = null;
    if (item) store.reschedule(item, withTime(day, minutes));
  }

  onMount(() => {
    // Start the view at 7am, or earlier if something is scheduled before then.
    const earliest = Math.min(7 * 60, ...columns.flatMap((c) => c.timed.map((t) => t.minutes - 30)));
    if (scroller) scroller.scrollTop = Math.max(0, (earliest / 60) * HOUR);
    const timer = setInterval(() => (now = new Date()), 60_000);
    return () => clearInterval(timer);
  });
</script>

<div class="grid" style:--days={days.length} style:--hour="{HOUR}px">
  <div class="head">
    <div class="gutter"></div>
    {#each columns as c (c.day)}
      <button
        class="dayname"
        class:today={c.day === today}
        onclick={() => {
          store.calDate = c.day;
          store.calView = "day";
          store.savePrefs();
        }}
      >
        <span class="wd">{c.date.toLocaleDateString(undefined, { weekday: days.length === 1 ? "long" : "short" })}</span>
        <span class="dn">{c.date.getDate()}</span>
      </button>
    {/each}
  </div>

  <div class="allday">
    <div class="gutter label">all-day</div>
    {#each columns as c (c.day)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="allday-col"
        class:drop={drop?.day === c.day && drop.minutes === null}
        ondragover={(e) => {
          e.preventDefault();
          drop = { day: c.day, minutes: null };
        }}
        ondrop={(e) => dropOnSlot(e, c.day)}
        ondblclick={() => (store.quickAddDue = c.day)}
      >
        {#each c.allDay as item (item.id)}
          <CalendarChip {item} />
        {/each}
      </div>
    {/each}
  </div>

  <div class="scroll" bind:this={scroller}>
    <div class="body">
      <div class="gutter hours">
        {#each Array(24) as _, h (h)}
          <div class="hour-label">{h === 0 ? "" : formatHour(h)}</div>
        {/each}
      </div>
      {#each columns as c (c.day)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="col"
          class:today={c.day === today}
          ondragover={(e) => {
            e.preventDefault();
            drop = { day: c.day, minutes: minutesAt(e, e.currentTarget as HTMLElement) };
          }}
          ondragleave={(e) => {
            if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) drop = null;
          }}
          ondrop={(e) => dropOnSlot(e, c.day)}
          ondblclick={(e) => (store.quickAddDue = withTime(c.day, minutesAt(e, e.currentTarget as HTMLElement)))}
          title="Double-click to add a to-do at this time"
        >
          {#if drop?.day === c.day && drop.minutes !== null}
            <div class="ghost" style:top="{(drop.minutes / 60) * HOUR}px"></div>
          {/if}
          {#each c.timed as p (p.item.id)}
            <div
              class="event"
              style:top="{(p.minutes / 60) * HOUR}px"
              style:height="{(SPAN / 60) * HOUR - 2}px"
              style:z-index={1 + p.indent}
              style:left="calc({p.indent * INDENT}px + (100% - {p.indent * INDENT}px) * {p.sub / p.subs} + 2px)"
              style:width="calc((100% - {p.indent * INDENT}px) / {p.subs} - 4px)"
            >
              <CalendarChip item={p.item} block />
            </div>
          {/each}
          {#if c.day === today}
            <div class="now" style:top="{(nowMinutes / 60) * HOUR}px"></div>
          {/if}
        </div>
      {/each}
    </div>
  </div>
</div>

<style>
  .grid {
    --gutter: 56px;
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .head,
  .allday,
  .body {
    display: grid;
    grid-template-columns: var(--gutter) repeat(var(--days), minmax(0, 1fr));
  }
  .head {
    padding-bottom: 6px;
  }
  /* Match the hour grid's scrollbar so the columns line up. */
  .head,
  .allday {
    padding-right: 10px;
  }
  .dayname {
    display: flex;
    align-items: baseline;
    justify-content: center;
    gap: 6px;
    padding: 4px 0;
    border-radius: 6px;
    color: var(--text-2);
  }
  .dayname:hover {
    background: var(--bg-hover);
  }
  .wd {
    font-size: 12px;
  }
  .dn {
    font-size: 17px;
    font-weight: 600;
    color: var(--text);
    font-variant-numeric: tabular-nums;
  }
  .dayname.today .dn {
    color: white;
    background: var(--danger);
    border-radius: 13px;
    padding: 0 7px;
  }
  .allday {
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
  }
  .label {
    font-size: 10.5px;
    color: var(--text-3);
    text-align: right;
    padding: 5px 8px 0 0;
  }
  .allday-col {
    min-width: 0;
    min-height: 28px;
    max-height: 104px;
    overflow-y: auto;
    padding: 3px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    border-left: 1px solid var(--border);
  }
  .allday-col.drop {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: scroll;
  }
  .body {
    height: calc(var(--hour) * 24);
  }
  .hours {
    position: relative;
  }
  .hour-label {
    height: var(--hour);
    font-size: 10.5px;
    color: var(--text-3);
    text-align: right;
    padding-right: 8px;
    transform: translateY(-7px);
    font-variant-numeric: tabular-nums;
  }
  .col {
    position: relative;
    border-left: 1px solid var(--border);
    background-image: linear-gradient(to bottom, var(--border) 1px, transparent 1px);
    background-size: 100% var(--hour);
  }
  .col.today {
    background-color: color-mix(in srgb, var(--accent) 2.5%, transparent);
  }
  .event {
    position: absolute;
    z-index: 1;
    border-radius: 4px;
    /* Separates stacked items. */
    box-shadow: 0 0 0 1px var(--bg);
  }
  .ghost {
    position: absolute;
    left: 2px;
    right: 2px;
    height: calc(var(--hour) / 2 - 2px);
    border-radius: 4px;
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    border: 1px dashed var(--accent);
    pointer-events: none;
  }
  .now {
    position: absolute;
    left: 0;
    right: 0;
    z-index: 2;
    border-top: 2px solid var(--danger);
    pointer-events: none;
  }
  .now::before {
    content: "";
    position: absolute;
    left: -4px;
    top: -5px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--danger);
  }
</style>
