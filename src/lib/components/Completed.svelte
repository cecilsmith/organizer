<script lang="ts">
  import { store, COMPLETED_DAYS } from "$lib/store.svelte";
  import Icon from "./Icon.svelte";
  import ItemRow from "./ItemRow.svelte";

  const tabName = $derived(
    store.tab === "all"
      ? ""
      : store.tab === "inbox"
        ? " in the Inbox"
        : ` in ${store.categories.find((c) => c.id === store.tab)?.name ?? ""}`,
  );
</script>

<div class="scroll" role="listbox" aria-label="Completed">
  {#if store.doneGroups.length === 0}
    <div class="empty">
      <Icon name="check" size={28} class="faint" />
      {#if store.query}
        <p class="muted">Nothing completed matches “{store.query}”.</p>
      {:else}
        <p class="muted">Nothing completed{tabName} yet.</p>
        <p class="faint small">Items you check off show up here, grouped by the day you finished them.</p>
      {/if}
    </div>
  {:else}
    {#each store.doneGroups as g (g.key)}
      <section class="group">
        <button class="group-head" onclick={() => store.toggleGroup(g.key)}>
          <Icon name={store.collapsed[g.key] ? "chevron" : "down"} size={12} class="faint" />
          <Icon name="check" size={13} />
          <span class="gtitle">{g.title}</span>
          <span class="faint">{g.items.length}</span>
        </button>
        {#if !store.collapsed[g.key]}
          {#each g.items as item (item.id)}
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
    <p class="note faint small">
      The last {COMPLETED_DAYS} days. Uncheck an item to reopen it — that goes back to Reminders, GitHub or GitLab too.
    </p>
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
  .gtitle {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
  .note {
    margin: 10px 12px 0;
    text-align: center;
  }
  .small {
    font-size: 12px;
  }
</style>
