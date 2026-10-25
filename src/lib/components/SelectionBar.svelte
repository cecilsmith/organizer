<script lang="ts">
  import { store, canDelete, todayStr } from "$lib/store.svelte";
  import Icon from "./Icon.svelte";

  const items = $derived(store.pickedItems);
  const toComplete = $derived(items.filter((it) => it.kind !== "pullRequest" && !it.completed));
  const toReopen = $derived(items.filter((it) => it.completed));
  const toReschedule = $derived(items.filter((it) => store.canReschedule(it)));
  const toDelete = $derived(items.filter((it) => canDelete(it)));

  let moveOpen = $state(false);
  let dateOpen = $state(false);
  let confirming = $state(false);

  function closeMenus() {
    moveOpen = false;
    dateOpen = false;
    confirming = false;
  }

  function move(category: string | null) {
    closeMenus();
    store.categorizePicked(category);
  }

  function schedule(due: string) {
    closeMenus();
    store.reschedulePicked(due);
  }
</script>

<svelte:window onclick={closeMenus} />

<div class="bar" role="toolbar" aria-label="Selected items">
  <span class="count">{store.pickedCount} selected</span>

  {#if toReopen.length && !toComplete.length}
    <button onclick={() => store.completePicked(false)}>
      <Icon name="sync" size={14} /> Reopen {toReopen.length}
    </button>
  {:else}
    <button
      disabled={!toComplete.length}
      title={toComplete.length ? "" : "Pull requests are closed on GitHub, not here"}
      onclick={() => store.completePicked(true)}
    >
      <Icon name="check" size={14} /> Complete {toComplete.length}
    </button>
  {/if}

  <div class="wrap">
    <button aria-haspopup="menu" aria-expanded={moveOpen} onclick={(e) => { e.stopPropagation(); const o = moveOpen; closeMenus(); moveOpen = !o; }}>
      <Icon name="tag" size={14} /> Move
    </button>
    {#if moveOpen}
      <div class="menu" role="menu">
        {#each store.categories as c (c.id)}
          <button role="menuitem" onclick={(e) => { e.stopPropagation(); move(c.id); }}>
            <span class="dot" style:background={c.color}></span>
            {c.name}
          </button>
        {/each}
        <hr />
        <button role="menuitem" onclick={(e) => { e.stopPropagation(); move(null); }}>
          <Icon name="inbox" size={13} /> Inbox (no area)
        </button>
      </div>
    {/if}
  </div>

  <div class="wrap">
    <button
      aria-haspopup="menu"
      aria-expanded={dateOpen}
      disabled={!toReschedule.length}
      title={toReschedule.length ? "" : "These dates come from a milestone or iteration"}
      onclick={(e) => { e.stopPropagation(); const o = dateOpen; closeMenus(); dateOpen = !o; }}
    >
      <Icon name="calendar" size={14} /> Reschedule
      {#if toReschedule.length < items.length}<span class="some">{toReschedule.length}</span>{/if}
    </button>
    {#if dateOpen}
      <div class="menu" role="menu">
        <button role="menuitem" onclick={(e) => { e.stopPropagation(); schedule(todayStr()); }}>Today</button>
        <button role="menuitem" onclick={(e) => { e.stopPropagation(); schedule(todayStr(1)); }}>Tomorrow</button>
        <button role="menuitem" onclick={(e) => { e.stopPropagation(); schedule(todayStr(7)); }}>Next week</button>
        <hr />
        <label class="pick">
          Pick a day
          <input type="date" onclick={(e) => e.stopPropagation()} onchange={(e) => schedule(e.currentTarget.value)} />
        </label>
        <button role="menuitem" onclick={(e) => { e.stopPropagation(); schedule(""); }}>
          <Icon name="x" size={13} /> No date
        </button>
      </div>
    {/if}
  </div>

  {#if confirming}
    <span class="confirm">
      Delete {toDelete.length}? Reminders go for good; issues close as not planned. This can't be undone.
      <button class="danger" onclick={(e) => { e.stopPropagation(); confirming = false; store.deletePicked(); }}>Delete</button>
      <button onclick={(e) => { e.stopPropagation(); confirming = false; }}>Cancel</button>
    </span>
  {:else}
    <button
      class="danger"
      disabled={!toDelete.length}
      title={toDelete.length ? "" : "Drafts and pull requests can't be deleted here"}
      onclick={(e) => { e.stopPropagation(); closeMenus(); confirming = true; }}
    >
      <Icon name="trash" size={14} /> Delete
      {#if toDelete.length < items.length}<span class="some">{toDelete.length}</span>{/if}
    </button>
  {/if}

  <button class="icon-btn clear" title="Clear selection (Esc)" aria-label="Clear selection" onclick={() => store.clearPicked()}>
    <Icon name="x" size={13} />
  </button>
</div>

<style>
  .bar {
    position: sticky;
    bottom: 12px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 8px auto 0;
    width: fit-content;
    max-width: calc(100% - 24px);
    padding: 7px 8px 7px 14px;
    border: 1px solid var(--border-strong);
    border-radius: 11px;
    background: var(--bg-elev);
    box-shadow: var(--shadow);
  }
  .count {
    font-size: 12px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .bar button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 9px;
    border-radius: 7px;
    white-space: nowrap;
  }
  .bar button:disabled {
    opacity: 0.4;
  }
  .bar button.danger:not(:disabled):hover {
    color: var(--danger);
  }
  /* How many of the selection an action actually applies to. */
  .some {
    font-size: 11px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  .confirm {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-2);
  }
  .wrap {
    position: relative;
  }
  .menu {
    position: absolute;
    bottom: calc(100% + 6px);
    left: 0;
    min-width: 200px;

    padding: 4px;
    border: 1px solid var(--border-strong);
    border-radius: 9px;
    background: var(--bg-elev);
    box-shadow: var(--shadow);
  }
  .menu button {
    width: 100%;
    justify-content: flex-start;
    height: 28px;
  }
  .menu hr {
    margin: 4px 2px;
    border: none;
    border-top: 1px solid var(--border);
  }
  .pick {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 4px 9px;
    font-size: 12px;
    white-space: nowrap;
    color: var(--text-2);
  }

  .pick input {
    height: 24px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .clear {
    margin-left: 2px;
  }
</style>
