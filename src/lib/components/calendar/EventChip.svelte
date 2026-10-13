<script lang="ts">
  import type { CalendarEvent } from "$lib/api";
  import { dueMinutes, formatTime, formatTimeShort } from "$lib/dates";
  import { store } from "$lib/store.svelte";

  let {
    event,
    block = false,
    /** For blocks: the minutes of this day the event covers. */
    segment = null,
    /** Show the start time (month grid). */
    showTime = true,
  }: {
    event: CalendarEvent;
    block?: boolean;
    segment?: { start: number; end: number } | null;
    showTime?: boolean;
  } = $props();

  const color = $derived(
    event.color ?? store.categories.find((c) => event.categories.includes(c.id))?.color ?? "var(--accent)",
  );
  const startMin = $derived(dueMinutes(event.start));
  const tall = $derived(!!segment && segment.end - segment.start >= 45);
  const range = $derived(
    segment ? `${formatTime(segment.start)} – ${formatTime(segment.end % 1440 || (segment.end ? 1440 : 0))}` : "",
  );
  const tooltip = $derived(
    [
      event.title,
      event.allDay ? "All day" : `${formatTime(startMin ?? 0)} – ${formatTime(dueMinutes(event.end) ?? 0)}`,
      event.location,
      event.calendarName,
      event.recurring ? "Repeats" : null,
      event.writable ? null : "Read-only calendar",
    ]
      .filter(Boolean)
      .join("\n"),
  );

  function ondragstart(e: DragEvent) {
    if (!event.writable || !e.dataTransfer) return;
    e.dataTransfer.setData("text/organizer-event", event.id);
    e.dataTransfer.effectAllowed = "move";
    store.selectedId = event.id;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div
  class="event"
  class:block
  class:allday={event.allDay}
  class:selected={store.selectedId === event.id}
  style:--c={color}
  draggable={event.writable}
  {ondragstart}
  onclick={(e) => {
    e.stopPropagation();
    store.selectedId = event.id;
  }}
  ondblclick={(e) => {
    e.stopPropagation();
    store.editEvent(event);
  }}
  title={tooltip}
>
  {#if block}
    <span class="title">{event.title}</span>
    {#if tall}
      <span class="sub">{range}</span>
      {#if event.location}<span class="sub">{event.location}</span>{/if}
    {/if}
  {:else}
    {#if !event.allDay}<span class="dot"></span>{/if}
    {#if showTime && !event.allDay && startMin !== null}<span class="time">{formatTimeShort(startMin)}</span>{/if}
    <span class="title">{event.title}</span>
  {/if}
</div>

<style>
  .event {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    height: 20px;
    padding: 0 6px;
    border-radius: 5px;
    font-size: 11.5px;
    line-height: 20px;
    color: var(--text);
    cursor: default;
  }
  .event[draggable="true"] {
    cursor: grab;
  }
  /* All-day events read as filled bars, timed ones as a dot + text (like Apple Calendar). */
  .event.allday {
    background: color-mix(in srgb, var(--c) 30%, transparent);
  }
  .event:not(.allday):not(.block):hover,
  .event.allday:hover {
    background: color-mix(in srgb, var(--c) 22%, transparent);
  }
  .event.selected:not(.block) {
    background: var(--c);
    color: white;
  }
  .dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--c);
  }
  .event.selected .dot {
    background: white;
  }
  .time {
    flex: none;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .event.selected .time {
    color: rgba(255, 255, 255, 0.85);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .event.block {
    height: 100%;
    flex-direction: column;
    align-items: stretch;
    gap: 0;
    padding: 1px 6px 0 7px;
    line-height: 14px;
    border-left: 3px solid var(--c);
    border-radius: 4px;
    background: color-mix(in srgb, var(--c) 24%, var(--bg));
    overflow: hidden;
  }
  .event.block .title {
    flex: none;
    font-weight: 600;
  }
  .event.block.selected {
    background: var(--c);
    color: white;
  }
  .sub {
    font-size: 11px;
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .event.block.selected .sub {
    color: rgba(255, 255, 255, 0.85);
  }
</style>
