<script lang="ts">
  import type { CalendarEvent, Item } from "$lib/api";
  import { dueMinutes, monthGrid, movedEvent, parseYmd, weekdayNames, withTime, ymd } from "$lib/dates";
  import { store } from "$lib/store.svelte";
  import CalendarChip from "./CalendarChip.svelte";
  import EventChip from "./EventChip.svelte";

  let { byDay, eventsByDay }: { byDay: Map<string, Item[]>; eventsByDay: Map<string, CalendarEvent[]> } = $props();

  type Entry = { key: string; item?: Item; event?: CalendarEvent };

  const anchor = $derived(parseYmd(store.calDate));
  const days = $derived(monthGrid(anchor, store.weekStart));
  const names = $derived(weekdayNames(store.weekStart));
  const today = $derived(ymd(new Date()));

  // Fit as many chips as the cell height allows; the rest collapse into "+N more".
  let gridHeight = $state(600);
  const CHIP = 22;
  const HEADER = 26;
  const maxChips = $derived(Math.max(1, Math.floor((gridHeight / 6 - HEADER - 4) / CHIP)));

  let dropDay = $state<string | null>(null);

  /** Events first (all-day, then by time), then to-dos — like Apple Calendar. */
  function entriesFor(day: string): Entry[] {
    const events = (eventsByDay.get(day) ?? []).map((event) => ({ key: event.id, event }));
    const items = (byDay.get(day) ?? []).map((item) => ({ key: item.id, item }));
    return [...events, ...items];
  }

  function openDay(day: string) {
    store.calDate = day;
    store.calView = "day";
    store.savePrefs();
  }

  function ondrop(e: DragEvent, day: string) {
    e.preventDefault();
    dropDay = null;
    const itemId = e.dataTransfer?.getData("text/organizer-item");
    const eventId = e.dataTransfer?.getData("text/organizer-event");
    const item = itemId && store.items.find((i) => i.id === itemId);
    // Moving to another day keeps the time of day.
    if (item) return store.reschedule(item, withTime(day, item.due ? dueMinutes(item.due) : null));
    const event = eventId && store.visibleEvents.find((ev) => ev.id === eventId);
    if (event && event.start.slice(0, 10) !== day) {
      store.updateEvent(event, movedEvent(event, day, event.allDay ? null : dueMinutes(event.start)), `move “${event.title}”`);
    }
  }
</script>

<div class="month">
  <div class="names">
    {#each names as n (n)}<div>{n}</div>{/each}
  </div>
  <div class="grid" bind:clientHeight={gridHeight}>
    {#each days as d (ymd(d))}
      {@const day = ymd(d)}
      {@const entries = entriesFor(day)}
      {@const overflow = entries.length > maxChips}
      {@const shown = overflow ? entries.slice(0, maxChips - 1) : entries}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="cell"
        class:other={d.getMonth() !== anchor.getMonth()}
        class:weekend={d.getDay() === 0 || d.getDay() === 6}
        class:drop={dropDay === day}
        ondragover={(e) => {
          e.preventDefault();
          dropDay = day;
        }}
        ondragleave={() => dropDay === day && (dropDay = null)}
        ondrop={(e) => ondrop(e, day)}
        ondblclick={() => (store.quickAddDue = day)}
        title="Double-click to add something on this day"
      >
        <button class="num" class:today={day === today} onclick={() => openDay(day)}>
          {d.getDate() === 1 ? d.toLocaleDateString(undefined, { month: "short", day: "numeric" }) : d.getDate()}
        </button>
        <div class="items">
          {#each shown as entry (entry.key)}
            {#if entry.event}
              <EventChip event={entry.event} />
            {:else if entry.item}
              <CalendarChip item={entry.item} />
            {/if}
          {/each}
          {#if overflow}
            <button class="more" onclick={() => openDay(day)}>+{entries.length - shown.length} more</button>
          {/if}
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .month {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .names {
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    padding: 0 0 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-3);
    text-align: right;
  }
  .names div {
    padding-right: 8px;
  }
  .grid {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: repeat(7, minmax(0, 1fr));
    grid-template-rows: repeat(6, minmax(0, 1fr));
    border-top: 1px solid var(--border);
    border-left: 1px solid var(--border);
  }
  .cell {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    padding: 2px 3px 3px;
    border-right: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
    display: flex;
    flex-direction: column;
  }
  .cell.weekend {
    background: color-mix(in srgb, var(--bg-input) 50%, transparent);
  }
  .cell.other .num {
    color: var(--text-3);
  }
  .cell.drop {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    outline: 2px solid color-mix(in srgb, var(--accent) 55%, transparent);
    outline-offset: -2px;
  }
  .num {
    align-self: flex-end;
    min-width: 22px;
    height: 22px;
    padding: 0 6px;
    border-radius: 11px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }
  .num:hover {
    background: var(--bg-hover);
  }
  .num.today {
    background: var(--danger);
    color: white;
    font-weight: 600;
  }
  .items {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
  }
  .more {
    align-self: flex-start;
    padding: 0 6px;
    height: 18px;
    border-radius: 4px;
    font-size: 11px;
    color: var(--text-2);
  }
  .more:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
</style>
