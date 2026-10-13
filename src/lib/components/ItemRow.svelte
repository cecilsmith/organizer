<script lang="ts">
  import type { Item } from "$lib/api";
  import { store, dueBucket, formatDue } from "$lib/store.svelte";
  import Icon from "./Icon.svelte";

  let {
    item,
    selected = false,
    showContainer = true,
    onselect,
  }: { item: Item; selected?: boolean; showContainer?: boolean; onselect: () => void } = $props();

  let expanded = $state(false);
  const menuOpen = $derived(store.menuFor === item.id);

  const cats = $derived(store.categories.filter((c) => item.categories.includes(c.id)));
  const color = $derived(cats[0]?.color ?? "var(--text-3)");
  const categoryTags = $derived(
    new Set(store.categories.flatMap((c) => [c.label.toLowerCase(), c.name.toLowerCase()])),
  );
  const labels = $derived(item.labels.filter((l) => !categoryTags.has(l.name.toLowerCase())));
  const bucket = $derived(dueBucket(item.due));
  const canComplete = $derived(item.kind !== "pullRequest");
  const kindIcon = $derived(
    item.kind === "reminder" ? "list" : item.kind === "pullRequest" ? "pr" : item.kind === "draftIssue" ? "draft" : "issue",
  );
  const priority = $derived(
    item.priority === 0 ? null : item.priority <= 4 ? "!!!" : item.priority === 5 ? "!!" : "!",
  );

  function move(cat: string | null) {
    store.menuFor = null;
    store.setCategory(item, cat);
  }

  /** Label text color that stays readable on GitHub's label background. */
  function labelStyle(hex: string) {
    if (!/^[0-9a-f]{6}$/i.test(hex)) return "";
    return `--label: #${hex}`;
  }
</script>

<div
  class="row"
  class:selected
  class:done={item.completed}
  role="option"
  aria-selected={selected}
  tabindex="-1"
  onclick={() => {
    onselect();
    if (item.body) expanded = !expanded;
  }}
  ondblclick={() => store.edit(item)}
  onkeydown={() => {}}
>
  {#if canComplete}
    <button
      class="check"
      style:--c={color}
      class:checked={item.completed}
      onclick={(e) => {
        e.stopPropagation();
        store.toggle(item);
      }}
      title={item.completed ? "Mark as not done" : item.kind === "reminder" ? "Complete" : item.kind === "draftIssue" ? "Archive draft" : "Close issue"}
      aria-label="Toggle complete"
    >
      {#if item.completed}<Icon name="check" size={11} />{/if}
    </button>
  {:else}
    <span class="check pr" title="Pull request"><Icon name="pr" size={13} /></span>
  {/if}

  <div class="main">
    <div class="title selectable">
      {#if priority}<span class="priority">{priority}</span>{/if}
      {item.title}
    </div>
    <div class="meta">
      {#if showContainer}
        <span class="container">
          <Icon name={kindIcon} size={12} />
          {item.containerName}{#if item.number}<span class="faint">#{item.number}</span>{/if}
        </span>
      {:else if item.number}
        <span class="faint">#{item.number}</span>
      {/if}
      {#if item.due}
        <span class="due {bucket}"><Icon name="calendar" size={12} />{formatDue(item.due)}</span>
      {/if}
      {#if item.status}<span class="status">{item.status}</span>{/if}
      {#each labels as l (l.name)}
        <span class="label" style={labelStyle(l.color)}>{l.name}</span>
      {/each}
      {#if item.assignees.length}
        <span class="faint">@{item.assignees.join(", @")}</span>
      {/if}
      {#if store.tab === "all" || store.tab === "inbox"}
        {#each cats as c (c.id)}
          <span class="cat" style:--c={c.color}>{c.name}</span>
        {/each}
      {/if}
    </div>
    {#if expanded && item.body}
      <div class="body selectable">{item.body}</div>
    {/if}
  </div>

  <div class="actions">
    <button
      class="icon-btn"
      title="Edit (↵)"
      onclick={(e) => {
        e.stopPropagation();
        store.edit(item);
      }}
    >
      <Icon name="pencil" />
    </button>
    <div class="menu-wrap">
      <button
        class="icon-btn"
        title="Move to area (M)"
        onclick={(e) => {
          e.stopPropagation();
          store.menuFor = menuOpen ? null : item.id;
        }}
      >
        <Icon name="tag" />
      </button>
      {#if menuOpen}
        <div class="menu" role="menu">
          {#each store.categories as c (c.id)}
            <button role="menuitem" onclick={(e) => { e.stopPropagation(); move(c.id); }}>
              <span class="dot" style:background={c.color}></span>
              {c.name}
              {#if item.categories.includes(c.id)}<Icon name="check" size={13} class="tick" />{/if}
            </button>
          {/each}
          <hr />
          <button role="menuitem" onclick={(e) => { e.stopPropagation(); move(null); }}>
            <Icon name="inbox" size={13} /> Inbox (no area)
          </button>
        </div>
      {/if}
    </div>
    <button
      class="icon-btn"
      title={item.kind === "reminder" ? "Open in Reminders (O)" : "Open on GitHub (O)"}
      onclick={(e) => {
        e.stopPropagation();
        store.open(item);
      }}
    >
      <Icon name={item.kind === "reminder" ? "external" : "github"} />
    </button>
  </div>
</div>

<style>
  .row {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 7px 10px 7px 12px;
    border-radius: 8px;
    position: relative;
    transition: opacity 0.25s;
    outline: none;
  }
  .row:hover {
    background: var(--bg-hover);
  }
  .row.selected {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .row.done .title {
    color: var(--text-3);
    text-decoration: line-through;
  }
  .row.done {
    opacity: 0.7;
  }
  .check {
    flex: none;
    margin-top: 1px;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    border: 1.5px solid color-mix(in srgb, var(--c) 75%, var(--text-3));
    display: grid;
    place-items: center;
    color: white;
  }
  .check:hover {
    background: color-mix(in srgb, var(--c) 18%, transparent);
  }
  .check.checked {
    background: var(--c);
    border-color: var(--c);
  }
  .check.pr {
    border: none;
    color: var(--ok);
  }
  .main {
    flex: 1;
    min-width: 0;
  }
  .title {
    line-height: 19px;
    overflow-wrap: anywhere;
  }
  .priority {
    color: var(--warn);
    font-weight: 700;
    margin-right: 2px;
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
  .container,
  .due {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .container .faint {
    margin-left: 3px;
  }
  .due.overdue {
    color: var(--danger);
  }
  .due.today {
    color: var(--accent);
  }
  .status {
    padding: 0 6px;
    border-radius: 4px;
    background: var(--bg-active);
  }
  .label {
    --label: var(--text-3);
    padding: 0 7px;
    border-radius: 10px;
    border: 1px solid color-mix(in srgb, var(--label) 60%, transparent);
    background: color-mix(in srgb, var(--label) 18%, transparent);
    color: color-mix(in srgb, var(--label) 55%, var(--text));
    line-height: 16px;
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
  .body {
    margin-top: 6px;
    padding: 8px 10px;
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text-2);
    font-size: 12px;
    white-space: pre-wrap;
    max-height: 220px;
    overflow: auto;
  }
  .actions {
    display: flex;
    gap: 2px;
    opacity: 0;
    margin: -3px 0;
  }
  .row:hover .actions,
  .row.selected .actions,
  .actions:has(.menu) {
    opacity: 1;
  }
  .menu-wrap {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    top: 28px;
    z-index: 20;
    min-width: 190px;
    padding: 5px;
    border-radius: 8px;
    background: var(--bg-elev);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 26px;
    padding: 0 8px;
    border-radius: 5px;
    text-align: left;
  }
  .menu button:hover {
    background: var(--accent);
    color: var(--accent-text);
  }
  .menu :global(.tick) {
    margin-left: auto;
  }
  .menu hr {
    border: none;
    border-top: 1px solid var(--border);
    margin: 4px 2px;
  }
</style>
