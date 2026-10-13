<script lang="ts">
  import { onMount, tick } from "svelte";
  import type { Item } from "$lib/api";
  import { store, dueBucket, type DueBucket, type GroupBy, type SourceFilter } from "$lib/store.svelte";
  import Icon from "$lib/components/Icon.svelte";
  import ItemRow from "$lib/components/ItemRow.svelte";
  import EditPanel from "$lib/components/EditPanel.svelte";
  import QuickAdd from "$lib/components/QuickAdd.svelte";
  import Settings from "$lib/components/Settings.svelte";
  import Sidebar from "$lib/components/Sidebar.svelte";

  let addInput = $state<HTMLInputElement>();
  let searchInput = $state<HTMLInputElement>();

  onMount(async () => {
    if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
      (await import("$lib/dev/mock")).installMock();
    }
    store.init();
  });

  const tabName = $derived(
    store.tab === "all"
      ? "All"
      : store.tab === "inbox"
        ? "Inbox"
        : (store.categories.find((c) => c.id === store.tab)?.name ?? "All"),
  );
  const tabColor = $derived(store.categories.find((c) => c.id === store.tab)?.color);

  const connected = $derived(
    !!store.snap && (store.snap.remindersAccess === "granted" || store.snap.accounts.length > 0),
  );

  interface Group {
    key: string;
    title: string;
    icon: string;
    color?: string | null;
    items: Item[];
    /** `items` in display order, with sub-issues placed under their parents. */
    rows: Row[];
  }

  interface Row {
    item: Item;
    depth: number;
    /** The parent's title when the parent isn't shown directly above. */
    parentTitle: string | null;
  }

  const byId = $derived(new Map(store.items.map((it) => [it.id, it])));

  /**
   * Nests sub-issues under their parent when both are in the same group.
   * Children whose parent is elsewhere (another group, filtered out, or not
   * tracked) stay at the top level with a "↳ parent" hint.
   */
  function nest(items: Item[], nested: boolean): Row[] {
    const here = new Set(items.map((i) => i.id));
    const children = new Map<string, Item[]>();
    const roots: Item[] = [];
    for (const it of items) {
      if (nested && it.parentId && here.has(it.parentId)) {
        const list = children.get(it.parentId) ?? [];
        list.push(it);
        children.set(it.parentId, list);
      } else {
        roots.push(it);
      }
    }
    const rows: Row[] = [];
    const seen = new Set<string>();
    const visit = (it: Item, depth: number) => {
      if (seen.has(it.id)) return; // guards against cycles
      seen.add(it.id);
      const nestedHere = depth > 0;
      rows.push({
        item: it,
        depth,
        parentTitle: !nestedHere && it.parentId ? (byId.get(it.parentId)?.title ?? null) : null,
      });
      for (const c of children.get(it.id) ?? []) visit(c, depth + 1);
    };
    for (const r of roots) visit(r, 0);
    // Anything left over is part of a parent cycle; show it flat.
    for (const it of items) if (!seen.has(it.id)) visit(it, 0);
    return rows;
  }

  const bucketNames: Record<DueBucket, string> = {
    overdue: "Overdue",
    today: "Today",
    tomorrow: "Tomorrow",
    week: "Next 7 days",
    later: "Later",
    none: "No date",
  };

  const groups: Group[] = $derived.by(() => {
    const map = new Map<string, Group>();
    if (store.groupBy === "due") {
      for (const b of Object.keys(bucketNames) as DueBucket[])
        map.set(b, { key: b, title: bucketNames[b], icon: "calendar", items: [], rows: [] });
      for (const it of store.visible) map.get(dueBucket(it.due))!.items.push(it);
      return [...map.values()]
        .filter((g) => g.items.length)
        .map((g) => ({ ...g, rows: nest(g.items, false) }));
    }
    const lists = new Map(store.snap?.reminderLists.map((l) => [l.id, l]) ?? []);
    for (const it of store.visible) {
      const key = it.kind === "reminder" ? `rem:${it.containerId}` : `gh:${it.containerName}`;
      let g = map.get(key);
      if (!g) {
        g = {
          key,
          title: it.containerName,
          icon: it.kind === "reminder" ? "list" : it.kind === "draftIssue" ? "stack" : "github",
          color: it.kind === "reminder" ? lists.get(it.containerId)?.color : null,
          items: [],
          rows: [],
        };
        map.set(key, g);
      }
      g.items.push(it);
    }
    return [...map.values()]
      .sort((a, b) => {
        const ar = a.key.startsWith("rem:") ? 0 : 1;
        const br = b.key.startsWith("rem:") ? 0 : 1;
        return ar - br || a.title.localeCompare(b.title);
      })
      .map((g) => ({ ...g, rows: nest(g.items, true) }));
  });

  const flat = $derived(groups.flatMap((g) => (collapsed[g.key] ? [] : g.rows.map((r) => r.item))));
  let collapsed = $state<Record<string, boolean>>({});

  function setTab(tab: string) {
    store.tab = tab;
    store.selectedId = null;
    store.savePrefs();
  }

  async function moveSelection(delta: number) {
    if (!flat.length) return;
    const i = flat.findIndex((it) => it.id === store.selectedId);
    const next = i < 0 ? (delta > 0 ? 0 : flat.length - 1) : Math.min(flat.length - 1, Math.max(0, i + delta));
    store.selectedId = flat[next].id;
    await tick();
    document.getElementById(`row-${CSS.escape(flat[next].id)}`)?.scrollIntoView({ block: "nearest" });
  }

  function onkeydown(e: KeyboardEvent) {
    // The editor panel handles its own keys.
    if (store.editing) return;
    const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement || e.target instanceof HTMLSelectElement;
    // ⌘Z undoes the last change and ⌘⇧Z redoes it (Ctrl works too). While
    // you're typing, they undo/redo text instead.
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z") {
      if (typing || store.settingsOpen) return;
      e.preventDefault();
      if (e.shiftKey) store.redo();
      else store.undo();
      return;
    }
    if (e.metaKey) {
      const n = Number(e.key);
      if (n >= 1 && n <= 9) {
        e.preventDefault();
        const tabs = ["all", "inbox", ...store.categories.map((c) => c.id)];
        if (tabs[n - 1]) setTab(tabs[n - 1]);
      } else if (e.key === "n") {
        e.preventDefault();
        addInput?.focus();
      } else if (e.key === "f") {
        e.preventDefault();
        searchInput?.focus();
      } else if (e.key === "r") {
        e.preventDefault();
        store.sync();
      } else if (e.key === ",") {
        e.preventDefault();
        store.openSettings();
      }
      return;
    }
    if (typing || store.settingsOpen) return;
    const selected = flat.find((it) => it.id === store.selectedId);
    switch (e.key) {
      case "ArrowDown":
      case "j":
        e.preventDefault();
        moveSelection(1);
        break;
      case "ArrowUp":
      case "k":
        e.preventDefault();
        moveSelection(-1);
        break;
      case " ":
      case "x":
        if (selected && selected.kind !== "pullRequest") {
          e.preventDefault();
          store.toggle(selected);
        }
        break;
      case "Enter":
      case "e":
        if (selected) {
          e.preventDefault();
          store.edit(selected);
        }
        break;
      case "o":
        if (selected) store.open(selected);
        break;
      case "m":
        if (selected) store.menuFor = selected.id;
        break;
      case "n":
        e.preventDefault();
        addInput?.focus();
        break;
      case "/":
        e.preventDefault();
        searchInput?.focus();
        break;
      case "Escape":
        store.menuFor = null;
        store.selectedId = null;
        break;
    }
  }

  function setSource(s: SourceFilter) {
    store.sourceFilter = s;
    store.savePrefs();
  }
  function setGroup(g: GroupBy) {
    store.groupBy = g;
    store.savePrefs();
  }
</script>

<svelte:window {onkeydown} onclick={() => (store.menuFor = null)} />

<div class="app">
  <Sidebar />

  <main>
    <header data-tauri-drag-region>
      <h1 data-tauri-drag-region>
        {#if tabColor}<span class="dot big" style:background={tabColor}></span>{/if}
        {tabName}
      </h1>
      <div class="search">
        <Icon name="search" size={14} />
        <input
          bind:this={searchInput}
          bind:value={store.query}
          placeholder="Search"
          onkeydown={(e) => {
            if (e.key === "Escape") {
              store.query = "";
              e.currentTarget.blur();
            }
          }}
        />
      </div>
      <div class="seg" role="group" aria-label="Source">
        {#each [["all", "All"], ["reminders", "Reminders"], ["github", "GitHub"]] as [v, label] (v)}
          <button class:on={store.sourceFilter === v} onclick={() => setSource(v as SourceFilter)}>{label}</button>
        {/each}
      </div>
      <div class="seg" role="group" aria-label="Group by">
        <button class:on={store.groupBy === "source"} onclick={() => setGroup("source")} title="Group by list / repo"><Icon name="list" size={14} /></button>
        <button class:on={store.groupBy === "due"} onclick={() => setGroup("due")} title="Group by due date"><Icon name="calendar" size={14} /></button>
      </div>
    </header>

    <QuickAdd bind:inputEl={addInput} />

    <div class="scroll" role="listbox" aria-label="To-dos">
      {#if store.snap && !connected}
        <div class="welcome">
          <h2>Everything you need to do, in one place.</h2>
          <p class="muted">
            Connect Apple Reminders and as many GitHub accounts, repos and projects as you like. Sort it all into
            Personal, Business, School and Fun with labels.
          </p>
          <div class="cards">
            <button class="card" onclick={() => store.openSettings("reminders")}>
              <Icon name="list" size={22} />
              <strong>Connect Apple Reminders</strong>
              <span class="muted">Live two-way sync with every list.</span>
            </button>
            <button class="card" onclick={() => store.openSettings("accounts")}>
              <Icon name="github" size={22} />
              <strong>Link GitHub</strong>
              <span class="muted">Import from the gh CLI or paste a token.</span>
            </button>
          </div>
        </div>
      {:else if store.snap && flat.length === 0}
        <div class="empty">
          {#if store.query}
            <p class="muted">Nothing matches “{store.query}”.</p>
          {:else}
            <Icon name="check" size={28} class="faint" />
            <p class="muted">All clear{store.tab !== "all" ? ` in ${tabName}` : ""}.</p>
            {#if store.tab === "inbox"}
              <p class="faint small">Items without an area land here. Give lists and repos a default area in Settings.</p>
            {:else if store.tab !== "all"}
              <p class="faint small">Items labeled <code>{store.categories.find((c) => c.id === store.tab)?.label}</code>, or from sources that default to {tabName}, show up here.</p>
            {/if}
          {/if}
        </div>
      {:else}
        {#each groups as g (g.key)}
          <section class="group">
            <button class="group-head" onclick={() => (collapsed[g.key] = !collapsed[g.key])}>
              <Icon name={collapsed[g.key] ? "chevron" : "down"} size={12} class="faint" />
              {#if g.color}<span class="dot" style:background={g.color}></span>{:else}<Icon name={g.icon} size={13} />{/if}
              <span class="gtitle">{g.title}</span>
              <span class="faint">{g.items.filter((i) => !i.completed).length}</span>
            </button>
            {#if !collapsed[g.key]}
              {#each g.rows as { item, depth, parentTitle } (item.id)}
                <div id="row-{item.id}">
                  <ItemRow
                    {item}
                    {depth}
                    {parentTitle}
                    selected={store.selectedId === item.id}
                    showContainer={store.groupBy !== "source"}
                    onselect={() => (store.selectedId = store.selectedId === item.id ? null : item.id)}
                  />
                </div>
              {/each}
            {/if}
          </section>
        {/each}
      {/if}
    </div>
  </main>
</div>

{#if store.settingsOpen}
  <Settings />
{/if}

{#if store.editing}
  {#key store.editing.id}
    <EditPanel item={store.editing} />
  {/key}
{/if}

<div class="toasts">
  {#each store.toasts as t (t.id)}
    <div class="toast {t.kind}" role="status">
      <span class="selectable">{t.text}</span>
      {#if t.action}
        <button
          class="btn ghost toast-action"
          onclick={() => {
            store.dismiss(t.id);
            t.action?.run();
          }}>{t.action.label}</button
        >
      {/if}
      <button class="icon-btn" onclick={() => store.dismiss(t.id)} aria-label="Dismiss"><Icon name="x" size={12} /></button>
    </div>
  {/each}
</div>

<style>
  .app {
    display: flex;
    height: 100vh;
  }
  main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  header {
    height: 52px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 20px;
  }
  h1 {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 9px;
    margin: 0;
    font-size: 19px;
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  .dot.big {
    width: 11px;
    height: 11px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    width: 220px;
    padding: 0 9px;
    border-radius: 7px;
    background: var(--bg-input);
    color: var(--text-3);
  }
  .search:focus-within {
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--accent) 45%, transparent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: none;
    outline: none;
    color: var(--text);
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: 7px;
    background: var(--bg-input);
  }
  .seg button {
    display: grid;
    place-items: center;
    height: 24px;
    padding: 0 9px;
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
  .small {
    font-size: 12px;
  }
  code {
    font-family: var(--mono);
    font-size: 11.5px;
  }
  .welcome {
    max-width: 560px;
    margin: 60px auto 0;
    text-align: center;
  }
  .welcome h2 {
    font-size: 22px;
    margin: 0 0 8px;
  }
  .cards {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 14px;
    margin-top: 24px;
  }
  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 22px 16px;
    border-radius: 12px;
    border: 1px solid var(--border);
    background: var(--bg-elev);
    text-align: center;
  }
  .card:hover {
    border-color: var(--accent);
  }
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 16px;
    z-index: 100;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 420px;
  }
  .toast {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 9px 8px 9px 12px;
    border-radius: 9px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    animation: rise 0.18s ease-out;
  }
  .toast span {
    flex: 1;
    padding-top: 3px;
  }
  .toast-action {
    height: 22px;
    color: var(--accent);
  }
  .toast.error {
    border-left: 3px solid var(--danger);
  }
  .toast.info {
    border-left: 3px solid var(--ok);
  }
  @keyframes rise {
    from {
      transform: translateY(8px);
      opacity: 0;
    }
  }
</style>
