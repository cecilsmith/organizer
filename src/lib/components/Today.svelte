<script lang="ts">
  import type { CalendarEvent } from "$lib/api";
  import { dueMinutes, formatTime } from "$lib/dates";
  import { store, todayStr } from "$lib/store.svelte";
  import Icon from "./Icon.svelte";
  import ItemRow from "./ItemRow.svelte";

  /**
   * An event's area colour, since this view isn't scoped to one area. Events
   * that belong to no area keep their calendar's colour.
   */
  function color(e: CalendarEvent) {
    return store.categories.find((c) => e.categories.includes(c.id))?.color ?? e.color ?? "var(--accent)";
  }

  /** "9:00 AM – 9:30 AM", or just the part of a longer event that lands today. */
  function when(e: CalendarEvent) {
    const today = todayStr();
    const began = e.start.slice(0, 10) < today;
    const goesOn = e.end.slice(0, 10) > today;
    if (e.allDay || (began && goesOn)) return "All day";
    if (began) return `until ${formatTime(dueMinutes(e.end) ?? 0)}`;
    const from = formatTime(dueMinutes(e.start) ?? 0);
    return goesOn ? `${from} onwards` : `${from} – ${formatTime(dueMinutes(e.end) ?? 0)}`;
  }
</script>

{#snippet eventRow(e: CalendarEvent)}
  {@const cats = store.categories.filter((c) => e.categories.includes(c.id))}
  <div
    class="row"
    class:selected={store.selectedId === e.id}
    style:--c={color(e)}
    role="option"
    aria-selected={store.selectedId === e.id}
    tabindex="-1"
    onclick={() => (store.selectedId = store.selectedId === e.id ? null : e.id)}
    ondblclick={() => store.editEvent(e)}
    onkeydown={() => {}}
  >
    <span class="bar"></span>
    <span class="time" class:allday={e.allDay}>{when(e)}</span>
    <div class="main">
      <div class="title selectable">{e.title}</div>
      <div class="meta">
        {#if e.location}<span class="where">{e.location}</span>{/if}
        <span class="faint">{e.calendarName}</span>
        {#if e.recurring}<span class="faint">Repeats</span>{/if}
        {#each cats as c (c.id)}
          <span class="cat" style:--c={c.color}>{c.name}</span>
        {/each}
      </div>
    </div>
    <div class="actions">
      <button class="icon-btn" title="Edit (↵)" onclick={(ev) => { ev.stopPropagation(); store.editEvent(e); }}>
        <Icon name="pencil" />
      </button>
      <button class="icon-btn" title="Open in Calendar (O)" onclick={(ev) => { ev.stopPropagation(); store.openEvent(e); }}>
        <Icon name="external" />
      </button>
    </div>
  </div>
{/snippet}

<div class="scroll" role="listbox" aria-label="Today">
  {#if store.todaySections.length === 0}
    <div class="empty">
      <Icon name="sun" size={28} class="faint" />
      {#if store.query}
        <p class="muted">Nothing today matches “{store.query}”.</p>
      {:else}
        <p class="muted">Nothing on today.</p>
        <p class="faint small">No events, nothing due, nothing overdue. Enjoy it.</p>
      {/if}
    </div>
  {:else}
    {#each store.todaySections as s (s.key)}
      <section class="group">
        <button class="group-head" class:warn={s.key === "today:overdue"} onclick={() => store.toggleGroup(s.key)}>
          <Icon name={store.collapsed[s.key] ? "chevron" : "down"} size={12} class="faint" />
          <Icon name={s.icon} size={13} />
          <span class="gtitle">{s.title}</span>
          <span class="faint">{s.events.length + s.items.filter((i) => !i.completed).length}</span>
        </button>
        {#if !store.collapsed[s.key]}
          {#each s.events as e (e.id)}
            <div id="row-{e.id}">{@render eventRow(e)}</div>
          {/each}
          {#each s.items as item (item.id)}
            <div id="row-{item.id}">
              <ItemRow
                {item}
                selected={store.selectedId === item.id}
                onselect={() => (store.selectedId = store.selectedId === item.id ? null : item.id)}
              />
            </div>
          {/each}
        {/if}
      </section>
    {/each}
  {/if}
</div>

<style>
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 0 10px 40px;
  }
  .group {
    margin-bottom: 8px;
  }
  .group-head {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 8px 12px 5px;
    background: var(--bg);
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
    text-align: left;
  }
  .group-head.warn {
    color: var(--danger);
  }
  .gtitle {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 7px 10px 7px 12px;
    border-radius: 8px;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .bar {
    flex: none;
    width: 3px;
    height: 17px;
    margin-top: 1px;
    border-radius: 2px;
    background: var(--c);
  }
  .time {
    flex: none;
    width: 128px;
    line-height: 19px;
    font-size: 12px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .time.allday {
    font-variant-numeric: normal;
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .title {
    line-height: 19px;
    overflow-wrap: anywhere;
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 10px;
    margin-top: 2px;
    font-size: 11.5px;
    color: var(--text-2);
  }
  .meta:empty {
    display: none;
  }
  .where {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 280px;
  }
  .cat {
    color: color-mix(in srgb, var(--c) 80%, var(--text));
    font-weight: 500;
  }
  .cat::before {
    content: "";
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--c);
    margin-right: 4px;
    vertical-align: 1px;
  }
  .actions {
    display: flex;
    gap: 2px;
    opacity: 0;
    margin: -3px 0;
  }
  .row:hover .actions,
  .row.selected .actions {
    opacity: 1;
  }
  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: 90px;
    text-align: center;
    color: var(--text-3);
  }
  .empty p {
    margin: 8px 0 0;
    max-width: 380px;
  }
  .small {
    font-size: 12px;
  }
</style>
