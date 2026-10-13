<script lang="ts">
  import { onMount } from "svelte";
  import type { CalendarEvent, Item } from "$lib/api";
  import { dueMinutes, formatHour, formatTime, movedEvent, parseYmd, segmentOn, toLocalString, withTime, ymd } from "$lib/dates";
  import { store } from "$lib/store.svelte";
  import CalendarChip from "./CalendarChip.svelte";
  import EventChip from "./EventChip.svelte";

  let {
    days,
    byDay,
    eventsByDay,
  }: { days: Date[]; byDay: Map<string, Item[]>; eventsByDay: Map<string, CalendarEvent[]> } = $props();

  const HOUR = 56; // px per hour: 15 minutes ≈ one line of text
  /** Reminders have no duration; draw each as a half-hour block. */
  const SPAN = 30;
  const SNAP = 15; // minutes
  /** Horizontal indent for each overlapping, later-starting block. */
  const INDENT = 14;

  let scroller = $state<HTMLDivElement>();
  let now = $state(new Date());
  let drop = $state<{ day: string; minutes: number | null } | null>(null);

  const today = $derived(ymd(now));
  const nowMinutes = $derived(now.getHours() * 60 + now.getMinutes());

  interface Block {
    key: string;
    start: number;
    end: number;
    item?: Item;
    event?: CalendarEvent;
  }

  interface Placed extends Block {
    /** How many earlier, still-running blocks this one is drawn on top of. */
    indent: number;
    /** Position among blocks starting at the same minute, which share the width. */
    sub: number;
    subs: number;
  }

  /**
   * Like Apple Calendar: a block that starts while another is still running is
   * indented and drawn over it, so the earlier title stays readable. Blocks
   * that start at the same minute split the width instead.
   */
  function layout(blocks: Block[]): Placed[] {
    const sorted = [...blocks].sort((a, b) => a.start - b.start || b.end - a.end);
    const out: Placed[] = [];
    const ends: number[] = [];
    for (let i = 0; i < sorted.length; ) {
      const start = sorted[i].start;
      const group: Block[] = [];
      while (i < sorted.length && sorted[i].start === start) group.push(sorted[i++]);
      const indent = ends.filter((end) => end > start).length;
      ends.push(Math.max(...group.map((b) => b.end)));
      group.forEach((b, sub) => out.push({ ...b, indent, sub, subs: group.length }));
    }
    return out;
  }

  const columns = $derived(
    days.map((d) => {
      const day = ymd(d);
      const items = byDay.get(day) ?? [];
      const events = eventsByDay.get(day) ?? [];
      const blocks: Block[] = [];
      for (const item of items) {
        const m = dueMinutes(item.due!);
        if (m !== null) blocks.push({ key: item.id, start: m, end: m + SPAN, item });
      }
      for (const event of events) {
        const seg = segmentOn(event, day);
        // Short events still get room for one line.
        if (seg) blocks.push({ key: event.id, start: seg.start, end: Math.max(seg.end, seg.start + 20), event });
      }
      return {
        day,
        date: d,
        allDayEvents: events.filter((e) => e.allDay),
        allDay: items.filter((i) => dueMinutes(i.due!) === null),
        timed: layout(blocks),
      };
    }),
  );

  function minutesAt(e: MouseEvent | DragEvent, col: HTMLElement) {
    const y = e.clientY - col.getBoundingClientRect().top;
    const m = Math.floor(((y / HOUR) * 60) / SNAP) * SNAP;
    return Math.max(0, Math.min(24 * 60 - SNAP, m));
  }

  function dropOnSlot(e: DragEvent, day: string) {
    e.preventDefault();
    const minutes = drop?.minutes ?? null;
    drop = null;
    const itemId = e.dataTransfer?.getData("text/organizer-item");
    const eventId = e.dataTransfer?.getData("text/organizer-event");
    const item = itemId && store.items.find((i) => i.id === itemId);
    if (item) return store.reschedule(item, withTime(day, minutes));
    const event = eventId && store.visibleEvents.find((ev) => ev.id === eventId);
    if (event) {
      const to = movedEvent(event, day, minutes);
      if (to.start !== event.start || to.end !== event.end || to.allDay !== event.allDay)
        store.updateEvent(event, to, `move “${event.title}”`);
    }
  }

  // ---- resizing an event's end ---------------------------------------------------

  let resize = $state<{ event: CalendarEvent; day: string; start: number; end: number; originY: number; base: number } | null>(null);

  function startResize(e: PointerEvent, p: Placed, day: string) {
    if (!p.event?.writable) return;
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    resize = { event: p.event, day, start: p.start, end: p.end, originY: e.clientY, base: p.end };
  }

  function onpointermove(e: PointerEvent) {
    if (!resize) return;
    const delta = Math.round(((e.clientY - resize.originY) / HOUR) * 60 / SNAP) * SNAP;
    const end = Math.min(24 * 60, Math.max(resize.start + SNAP, resize.base + delta));
    if (end !== resize.end) resize = { ...resize, end };
  }

  function onpointerup() {
    if (!resize) return;
    const { event, day, end, base } = resize;
    resize = null;
    if (end === base) return;
    const endDate = parseYmd(day);
    endDate.setMinutes(end);
    store.updateEvent(event, { end: toLocalString(endDate) }, `resize “${event.title}”`);
  }

  onMount(() => {
    // Start the view at 7am, or earlier if something is scheduled before then.
    const earliest = Math.min(7 * 60, ...columns.flatMap((c) => c.timed.map((t) => t.start - 30)));
    if (scroller) scroller.scrollTop = Math.max(0, (earliest / 60) * HOUR);
    const timer = setInterval(() => (now = new Date()), 60_000);
    return () => clearInterval(timer);
  });
</script>

<svelte:window {onpointermove} {onpointerup} />

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
        {#each c.allDayEvents as event (event.id)}
          <EventChip {event} />
        {/each}
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
          title="Double-click to add something at this time"
        >
          {#if drop?.day === c.day && drop.minutes !== null}
            <div class="ghost" style:top="{(drop.minutes / 60) * HOUR}px"></div>
          {/if}
          {#each c.timed as p (p.key)}
            {@const end = resize && resize.event.id === p.key && resize.day === c.day ? resize.end : p.end}
            <div
              class="event"
              style:top="{(p.start / 60) * HOUR}px"
              style:height="{((end - p.start) / 60) * HOUR - 2}px"
              style:z-index={1 + p.indent}
              style:left="calc({p.indent * INDENT}px + (100% - {p.indent * INDENT}px) * {p.sub / p.subs} + 2px)"
              style:width="calc((100% - {p.indent * INDENT}px) / {p.subs} - 4px)"
            >
              {#if p.item}
                <CalendarChip item={p.item} block />
              {:else if p.event}
                <EventChip event={p.event} block segment={{ start: p.start, end }} />
                {#if p.event.writable}
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div class="resize" title="Drag to change the end time" onpointerdown={(e) => startResize(e, p, c.day)}></div>
                {/if}
              {/if}
            </div>
            {#if resize && resize.event.id === p.key && resize.day === c.day}
              <span class="resize-time" style:top="{(end / 60) * HOUR}px">{formatTime(end % 1440)}</span>
            {/if}
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
  .resize {
    position: absolute;
    left: 0;
    right: 0;
    bottom: -2px;
    height: 7px;
    cursor: ns-resize;
  }
  .resize-time {
    position: absolute;
    right: 4px;
    z-index: 20;
    transform: translateY(-50%);
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--text);
    color: var(--bg);
    font-size: 11px;
    pointer-events: none;
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
