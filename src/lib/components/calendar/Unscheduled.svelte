<script lang="ts">
  import type { Item } from "$lib/api";
  import { store } from "$lib/store.svelte";
  import CalendarChip from "./CalendarChip.svelte";

  let { items }: { items: Item[] } = $props();

  const reminders = $derived(items.filter((i) => i.kind === "reminder"));
  const github = $derived(items.filter((i) => i.kind !== "reminder"));
  let dropping = $state(false);
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<aside
  class="tray"
  class:drop={dropping}
  ondragover={(e) => {
    e.preventDefault();
    dropping = true;
  }}
  ondragleave={(e) => {
    if (!(e.currentTarget as HTMLElement).contains(e.relatedTarget as Node)) dropping = false;
  }}
  ondrop={(e) => {
    e.preventDefault();
    dropping = false;
    const id = e.dataTransfer?.getData("text/organizer-item");
    const item = store.items.find((i) => i.id === id);
    // Dropping here clears the due date.
    if (item?.due) store.reschedule(item, "");
  }}
>
  <header>
    <span>No date</span>
    <span class="faint">{items.length}</span>
  </header>
  <p class="faint hint">Drag items onto the calendar to schedule them.</p>
  <div class="list">
    {#each reminders as item (item.id)}
      <CalendarChip {item} />
    {/each}
    {#if github.length}
      <div class="sub faint">Issues</div>
      {#each github as item (item.id)}
        <CalendarChip {item} />
      {/each}
    {/if}
    {#if !items.length}
      <p class="faint hint">Everything here has a date.</p>
    {/if}
  </div>
</aside>

<style>
  .tray {
    width: 220px;
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 1px solid var(--border);
    padding: 0 0 0 12px;
  }
  .tray.drop {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  header {
    display: flex;
    justify-content: space-between;
    font-size: 12px;
    font-weight: 600;
    color: var(--text-2);
    padding: 2px 4px 0 0;
  }
  .hint {
    font-size: 11px;
    margin: 4px 0 8px;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding-right: 4px;
  }
  .sub {
    margin: 8px 0 2px;
    font-size: 11px;
    font-weight: 600;
  }
</style>
