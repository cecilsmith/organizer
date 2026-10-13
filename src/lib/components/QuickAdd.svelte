<script lang="ts">
  import { api, errorText, type NewItem } from "$lib/api";
  import { store, todayStr } from "$lib/store.svelte";
  import Icon from "./Icon.svelte";

  let { inputEl = $bindable() }: { inputEl?: HTMLInputElement } = $props();

  let title = $state("");
  let notes = $state("");
  let due = $state("");
  let time = $state("");
  let priority = $state(0);
  let projectId = $state("");
  let assignSelf = $state(false);
  let busy = $state(false);
  let expanded = $state(false);

  const snap = $derived(store.snap);
  const category = $derived(store.categories.some((c) => c.id === store.tab) ? store.tab : null);
  const categoryName = $derived(store.categories.find((c) => c.id === category)?.name);
  const lists = $derived(snap?.reminderLists.filter((l) => l.enabled && l.writable) ?? []);
  const repos = $derived(snap?.repos ?? []);
  const hasTargets = $derived(lists.length + repos.length > 0);

  // Remember the last target per tab; otherwise prefer a source mapped to this area.
  const targetKey = $derived(`organizer.target.${store.tab}`);
  let chosen = $state<Record<string, string>>({});

  function isValid(t: string | null | undefined): t is string {
    if (!t) return false;
    const [kind, id] = [t.slice(0, t.indexOf(":")), t.slice(t.indexOf(":") + 1)];
    return kind === "rem" ? lists.some((l) => l.id === id) : repos.some((r) => r.id === id);
  }

  const target = $derived.by(() => {
    const picked = chosen[store.tab];
    if (isValid(picked)) return picked;
    let saved: string | null = null;
    try {
      saved = localStorage.getItem(targetKey);
    } catch {}
    if (isValid(saved)) return saved;
    const list = lists.find((l) => l.defaultCategory === category);
    if (category && list) return `rem:${list.id}`;
    const repo = repos.find((r) => r.defaultCategory === category);
    if (category && repo) return `repo:${repo.id}`;
    if (lists[0]) return `rem:${lists[0].id}`;
    if (repos[0]) return `repo:${repos[0].id}`;
    return "";
  });

  const isReminder = $derived(target.startsWith("rem:"));
  const targetId = $derived(target.slice(target.indexOf(":") + 1));
  const repo = $derived(repos.find((r) => r.id === targetId));
  const projects = $derived(snap?.projects.filter((p) => repo && p.accountId === repo.accountId) ?? []);

  function pick(value: string) {
    chosen[store.tab] = value;
    try {
      localStorage.setItem(targetKey, value);
    } catch {}
  }

  function reset() {
    title = "";
    notes = "";
    due = "";
    time = "";
    priority = 0;
    expanded = false;
  }

  async function submit() {
    const t = title.trim();
    if (!t || busy || !target) return;
    busy = true;
    const req: NewItem = isReminder
      ? {
          target: "reminder",
          listId: targetId,
          title: t,
          notes: notes || null,
          due: due ? (time ? `${due}T${time}` : due) : null,
          priority,
          category,
        }
      : {
          target: "issue",
          repoId: targetId,
          title: t,
          body: notes || null,
          category,
          projectId: projectId || null,
          assignSelf,
        };
    try {
      const item = await api.createItem(req);
      // Undo deletes a new reminder, so redo recreates it (with a new id). Undo
      // closes a new issue as "not planned", so redo just reopens it.
      let currentId = item.id;
      const entry = store.record(
        `add “${item.title}”`,
        () => api.discardItem(currentId),
        async () => {
          if (item.kind === "reminder") currentId = (await api.createItem(req)).id;
          else await api.setCompleted(currentId, false, item);
        },
      );
      store.toast(`Added “${item.title}” to ${item.containerName}`, "info", {
        label: "Undo",
        run: () => store.undo(entry),
      });
      reset();
      await store.refresh();
    } catch (e) {
      store.toast(errorText(e));
    } finally {
      busy = false;
    }
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.target === inputEl || e.metaKey)) {
      e.preventDefault();
      submit();
    } else if (e.key === "Escape") {
      reset();
      (e.target as HTMLElement).blur();
    }
  }
</script>

{#if hasTargets}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="quick" class:expanded={expanded || title} onkeydown={onkey}>
    <div class="line">
      <Icon name="plus" class="faint" />
      <input
        bind:this={inputEl}
        bind:value={title}
        class="title"
        placeholder={categoryName ? `New ${categoryName.toLowerCase()} to-do…` : "New to-do…"}
        disabled={busy}
        spellcheck="true"
      />
      <select class="field target" value={target} onchange={(e) => pick(e.currentTarget.value)} title="Where to create it">
        {#if lists.length}
          <optgroup label="Apple Reminders">
            {#each lists as l (l.id)}
              <option value={`rem:${l.id}`}>{l.title}</option>
            {/each}
          </optgroup>
        {/if}
        {#if repos.length}
          <optgroup label="GitHub issue in…">
            {#each repos as r (r.id)}
              <option value={`repo:${r.id}`}>{r.owner}/{r.name}</option>
            {/each}
          </optgroup>
        {/if}
      </select>
      <button class="btn primary" onclick={submit} disabled={!title.trim() || busy}>
        {busy ? "Adding…" : "Add"}
      </button>
    </div>

    {#if expanded || title}
      <div class="details">
        <textarea
          class="field"
          rows="2"
          bind:value={notes}
          placeholder={isReminder ? "Notes" : "Issue description (Markdown)"}
          onfocus={() => (expanded = true)}
        ></textarea>
        <div class="opts">
          {#if isReminder}
            <div class="chips">
              <button class:on={!due} onclick={() => (due = "")}>No date</button>
              <button class:on={due === todayStr()} onclick={() => (due = todayStr())}>Today</button>
              <button class:on={due === todayStr(1)} onclick={() => (due = todayStr(1))}>Tomorrow</button>
              <button class:on={due === todayStr(7)} onclick={() => (due = todayStr(7))}>Next week</button>
            </div>
            <input type="date" class="field" bind:value={due} />
            {#if due}<input type="time" class="field" bind:value={time} title="Optional time (adds an alert)" />{/if}
            <select class="field" bind:value={priority} title="Priority">
              <option value={0}>No priority</option>
              <option value={9}>! Low</option>
              <option value={5}>!! Medium</option>
              <option value={1}>!!! High</option>
            </select>
          {:else}
            {#if projects.length}
              <select class="field" bind:value={projectId} title="Also add to a project">
                <option value="">No project</option>
                {#each projects as p (p.id)}
                  <option value={p.id}>{p.title}</option>
                {/each}
              </select>
            {/if}
            <label class="check"><input type="checkbox" bind:checked={assignSelf} /> Assign to me</label>
            {#if category}
              <span class="faint hint"><Icon name="tag" size={12} /> label “{store.categories.find((c) => c.id === category)?.label}”</span>
            {/if}
          {/if}
          <span class="faint hint right">↵ to add · esc to clear</span>
        </div>
      </div>
    {/if}
  </div>
{:else if store.snap}
  <div class="quick empty">
    <span class="muted">Connect Apple Reminders or a GitHub repo to start adding to-dos.</span>
    <button class="btn" onclick={() => store.openSettings("reminders")}>Connect Reminders</button>
    <button class="btn" onclick={() => store.openSettings("accounts")}><Icon name="github" size={14} /> Link GitHub</button>
  </div>
{/if}

<style>
  .quick {
    margin: 0 20px 10px;
    padding: 6px 8px 6px 12px;
    border-radius: 10px;
    border: 1px solid var(--border);
    background: var(--bg-elev);
    transition: box-shadow 0.15s;
  }
  .quick:focus-within {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .quick.empty {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px;
  }
  .quick.empty span {
    flex: 1;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .title {
    flex: 1;
    min-width: 0;
    height: 28px;
    border: none;
    background: none;
    outline: none;
    font-size: 13.5px;
  }
  .target {
    max-width: 200px;
    height: 26px;
    color: var(--text-2);
  }
  .details {
    padding: 6px 0 4px 24px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .details textarea {
    width: 100%;
    min-height: 40px;
  }
  .opts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
  }
  .opts .field {
    height: 26px;
  }
  .chips {
    display: flex;
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .chips button {
    padding: 0 9px;
    height: 24px;
    font-size: 12px;
    color: var(--text-2);
    border-right: 1px solid var(--border);
  }
  .chips button:last-child {
    border-right: none;
  }
  .chips button:hover {
    background: var(--bg-hover);
  }
  .chips button.on {
    background: var(--bg-active);
    color: var(--text);
    font-weight: 500;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text-2);
  }
  .hint {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11.5px;
  }
  .right {
    margin-left: auto;
  }
</style>
