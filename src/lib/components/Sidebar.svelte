<script lang="ts">
  import { store, relativeTime } from "$lib/store.svelte";
  import Icon from "./Icon.svelte";

  const lastSynced = $derived(
    store.snap?.sources
      .map((s) => s.lastSynced)
      .filter(Boolean)
      .sort()
      .at(-1) ?? null,
  );

  function select(tab: string) {
    store.setTab(tab);
  }
</script>

<aside class="sidebar">
  <div class="titlebar" data-tauri-drag-region></div>

  <nav>
    <button
      class="tab"
      class:active={store.tab === "today"}
      onclick={() => select("today")}
      title="⌘1 · today's events, what's due and what's overdue, across every area"
    >
      <Icon name="sun" />
      <span class="name">Today</span>
      <span class="count">{store.counts.today || ""}</span>
    </button>
    <button class="tab" class:active={store.tab === "all"} onclick={() => select("all")}>
      <Icon name="stack" />
      <span class="name">All</span>
      <span class="count">{store.counts.all || ""}</span>
    </button>
    <button class="tab" class:active={store.tab === "inbox"} onclick={() => select("inbox")}>
      <Icon name="inbox" />
      <span class="name">Inbox</span>
      <span class="count">{store.counts.inbox || ""}</span>
    </button>

    <div class="heading">Areas</div>
    {#each store.categories as c, i (c.id)}
      <button
        class="tab"
        class:active={store.tab === c.id}
        onclick={() => select(c.id)}
        title={`⌘${i + 4} · items labeled “${c.label}”`}
      >
        <span class="dot" style:background={c.color}></span>
        <span class="name">{c.name}</span>
        <span class="count">{store.counts[c.id] || ""}</span>
      </button>
    {/each}
    <button class="tab add" onclick={() => store.openSettings("categories")}>
      <Icon name="plus" size={14} />
      <span class="name">New area</span>
    </button>
  </nav>

  <footer>
    {#if store.sourceErrors.length}
      <button
        class="status error"
        onclick={() => store.openSettings("status")}
        title={store.sourceErrors.map((s) => `${s.label}: ${s.error}`).join("\n")}
      >
        <Icon name="alert" size={14} />
        {store.sourceErrors.length} source{store.sourceErrors.length > 1 ? "s" : ""} failed
      </button>
    {:else}
      <span class="status faint">
        {store.syncing ? "Syncing…" : `Updated ${relativeTime(lastSynced)}`}
      </span>
    {/if}
    <button class="icon-btn" onclick={() => store.sync()} title="Sync now (⌘R)" disabled={store.syncing}>
      <Icon name="sync" class={store.syncing ? "spin" : ""} />
    </button>
    <button class="icon-btn" onclick={() => store.openSettings()} title="Settings (⌘,)">
      <Icon name="gear" />
    </button>
  </footer>
</aside>

<style>
  .sidebar {
    width: 220px;
    flex: none;
    display: flex;
    flex-direction: column;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
  }
  .titlebar {
    height: 46px;
    flex: none;
  }
  nav {
    flex: 1;
    overflow-y: auto;
    padding: 0 10px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .heading {
    margin: 14px 8px 4px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-3);
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 28px;
    padding: 0 8px;
    border-radius: 6px;
    color: var(--text);
    text-align: left;
  }
  .tab :global(.icon) {
    color: var(--text-2);
  }
  .tab .dot {
    margin: 0 3.5px;
  }
  .tab:hover {
    background: var(--bg-hover);
  }
  .tab.active {
    background: var(--bg-active);
    font-weight: 500;
  }
  .tab.add {
    color: var(--text-3);
  }
  .tab.add :global(.icon) {
    color: var(--text-3);
    margin: 0 1px;
  }
  .name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font-size: 12px;
    color: var(--text-3);
    font-variant-numeric: tabular-nums;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
  }
  .status {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
    overflow: hidden;
    white-space: nowrap;
  }
  .status.error {
    color: var(--danger);
  }
</style>
