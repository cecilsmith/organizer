<script lang="ts">
  import type { Item } from "$lib/api";
  import { dueMinutes, formatTime, formatTimeShort } from "$lib/dates";
  import { store, dueBucket } from "$lib/store.svelte";
  import Icon from "../Icon.svelte";

  let {
    item,
    showTime = true,
    block = false,
  }: {
    item: Item;
    /** Prefix timed items with their time (month view). Time-grid blocks skip it: position shows the time. */
    showTime?: boolean;
    /** Fill the parent (time-grid events) instead of sizing to content. */
    block?: boolean;
  } = $props();

  const color = $derived(store.categories.find((c) => item.categories.includes(c.id))?.color ?? "var(--text-3)");
  const minutes = $derived(item.due ? dueMinutes(item.due) : null);
  const overdue = $derived(!item.completed && dueBucket(item.due) === "overdue");
  const draggable = $derived(item.kind === "reminder" && !item.completed);
  const kindIcon = $derived(
    item.kind === "pullRequest" ? "pr" : item.kind === "draftIssue" ? "draft" : item.kind === "issue" ? "issue" : null,
  );

  function ondragstart(e: DragEvent) {
    if (!draggable || !e.dataTransfer) return;
    e.dataTransfer.setData("text/organizer-item", item.id);
    e.dataTransfer.effectAllowed = "move";
    store.selectedId = item.id;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<div
  class="chip"
  class:block
  class:selected={store.selectedId === item.id}
  class:done={item.completed}
  class:overdue
  style:--c={color}
  {draggable}
  {ondragstart}
  onclick={(e) => {
    e.stopPropagation();
    store.selectedId = item.id;
  }}
  ondblclick={(e) => {
    e.stopPropagation();
    store.edit(item);
  }}
  title="{minutes !== null ? `${formatTime(minutes)} · ` : ''}{item.title}{item.containerName ? ` — ${item.containerName}` : ''}{item.kind !== 'reminder' ? '\n(GitHub dates are set on GitHub)' : ''}"
>
  {#if item.kind !== "pullRequest"}
    <button
      class="check"
      class:checked={item.completed}
      aria-label="Toggle complete"
      onclick={(e) => {
        e.stopPropagation();
        store.toggle(item);
      }}
    >
      {#if item.completed}<Icon name="check" size={8} />{/if}
    </button>
  {/if}
  {#if kindIcon}<Icon name={kindIcon} size={11} class="kind" />{/if}
  {#if showTime && !block && minutes !== null}<span class="time">{formatTimeShort(minutes)}</span>{/if}
  <span class="title">{item.title}</span>
</div>

<style>
  .chip {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    height: 20px;
    padding: 0 6px 0 4px;
    border-radius: 5px;
    font-size: 11.5px;
    line-height: 20px;
    background: color-mix(in srgb, var(--c) 16%, transparent);
    color: var(--text);
    cursor: default;
  }
  .chip[draggable="true"] {
    cursor: grab;
  }
  .chip:hover {
    background: color-mix(in srgb, var(--c) 26%, transparent);
  }
  .chip.block:hover {
    background: color-mix(in srgb, var(--c) 32%, var(--bg));
  }
  .chip.selected {
    background: var(--c);
    color: white;
  }
  .chip.selected .check {
    border-color: white;
  }
  .chip.block {
    /* Opaque, so items stacked on top of each other stay legible. */
    background: color-mix(in srgb, var(--c) 22%, var(--bg));
    height: 100%;
    align-items: flex-start;
    padding-top: 0;
    line-height: 14px;
    border-left: 3px solid var(--c);
    border-radius: 4px;
  }
  .chip.block.selected {
    background: var(--c);
  }
  .chip.done .title {
    text-decoration: line-through;
    opacity: 0.6;
  }
  .chip.overdue:not(.selected) .time {
    color: var(--danger);
  }
  .check {
    flex: none;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 1.3px solid var(--c);
    display: grid;
    place-items: center;
    color: white;
    margin-top: 0;
  }
  .chip.block .check {
    margin-top: 1.5px;
  }
  .check.checked {
    background: var(--c);
  }
  .chip :global(.kind) {
    opacity: 0.75;
  }
  .chip.block :global(.kind) {
    margin-top: 1.5px;
  }
  .time {
    flex: none;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .chip.selected .time {
    color: rgba(255, 255, 255, 0.85);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
