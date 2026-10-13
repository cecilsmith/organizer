<script lang="ts">
  import type { Item } from "$lib/api";
  import { addDays, dueMinutes, parseYmd, weekDays } from "$lib/dates";
  import { store, type CalView } from "$lib/store.svelte";
  import Icon from "../Icon.svelte";
  import MonthView from "./MonthView.svelte";
  import TimeGrid from "./TimeGrid.svelte";
  import Unscheduled from "./Unscheduled.svelte";

  const anchor = $derived(parseYmd(store.calDate));

  /** Visible items grouped by day: all-day first, then by time, then priority. */
  const byDay = $derived.by(() => {
    const map = new Map<string, Item[]>();
    for (const it of store.visible) {
      if (!it.due) continue;
      const day = it.due.slice(0, 10);
      const list = map.get(day) ?? [];
      list.push(it);
      map.set(day, list);
    }
    for (const list of map.values()) {
      list.sort((a, b) => {
        const ma = dueMinutes(a.due!) ?? -1;
        const mb = dueMinutes(b.due!) ?? -1;
        return ma - mb || (a.priority || 10) - (b.priority || 10) || a.title.localeCompare(b.title);
      });
    }
    return map;
  });
  const unscheduled = $derived(store.visible.filter((i) => !i.due));

  const title = $derived.by(() => {
    if (store.calView === "month") return anchor.toLocaleDateString(undefined, { month: "long", year: "numeric" });
    if (store.calView === "day")
      return anchor.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric", year: "numeric" });
    const days = weekDays(anchor, store.weekStart);
    const [a, b] = [days[0], days[6]];
    const sameMonth = a.getMonth() === b.getMonth();
    const start = a.toLocaleDateString(undefined, { month: "short", day: "numeric" });
    const end = b.toLocaleDateString(undefined, sameMonth ? { day: "numeric" } : { month: "short", day: "numeric" });
    return `${start} – ${end}, ${b.getFullYear()}`;
  });

  function setView(v: CalView) {
    store.calView = v;
    store.savePrefs();
  }
</script>

<div class="calendar">
  <div class="toolbar">
    <div class="nav">
      <button class="icon-btn" onclick={() => store.calShift(-1)} title="Previous (←)"><Icon name="chevron" class="flip" /></button>
      <button class="btn" onclick={() => store.calToday()} title="Today (T)">Today</button>
      <button class="icon-btn" onclick={() => store.calShift(1)} title="Next (→)"><Icon name="chevron" /></button>
    </div>
    <h2>{title}</h2>
    <div class="seg" role="group" aria-label="Calendar view">
      {#each [["day", "Day"], ["week", "Week"], ["month", "Month"]] as [v, label] (v)}
        <button class:on={store.calView === v} onclick={() => setView(v as CalView)}>{label}</button>
      {/each}
    </div>
    <button
      class="icon-btn"
      class:on={store.showUnscheduled}
      title={store.showUnscheduled ? "Hide items without a date" : "Show items without a date"}
      onclick={() => {
        store.showUnscheduled = !store.showUnscheduled;
        store.savePrefs();
      }}
    >
      <Icon name="inbox" />
    </button>
  </div>

  <div class="body">
    {#if store.calView === "month"}
      <MonthView {byDay} />
    {:else}
      {#key store.calView}
        <TimeGrid
          days={store.calView === "week" ? weekDays(anchor, store.weekStart) : [addDays(anchor, 0)]}
          {byDay}
        />
      {/key}
    {/if}
    {#if store.showUnscheduled}
      <Unscheduled items={unscheduled} />
    {/if}
  </div>
</div>

<style>
  .calendar {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 0 20px 14px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding-bottom: 10px;
  }
  .nav {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .nav :global(.flip) {
    transform: scaleX(-1);
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: 7px;
    background: var(--bg-input);
  }
  .seg button {
    height: 24px;
    padding: 0 12px;
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
  .icon-btn.on {
    background: var(--bg-active);
    color: var(--text);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    gap: 12px;
  }
</style>
