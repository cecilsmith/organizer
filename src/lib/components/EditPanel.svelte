<script lang="ts">
  import { onMount, tick } from "svelte";
  import { api, errorText, type Item, type ItemPatch, type Label } from "$lib/api";
  import { store, todayStr } from "$lib/store.svelte";
  import CategorySelect from "./CategorySelect.svelte";
  import Icon from "./Icon.svelte";
  import { hashtags, stripTags } from "$lib/tags";

  let { item }: { item: Item } = $props();

  // The panel is keyed by item id, so reading the initial prop once is intended.
  // svelte-ignore state_referenced_locally
  const initial = item;
  const isReminder = initial.kind === "reminder";
  const isGitHub = initial.kind === "issue" || initial.kind === "pullRequest";
  const isDraft = initial.kind === "draftIssue";

  interface Form {
    title: string;
    body: string;
    dueDate: string;
    dueTime: string;
    priority: number;
    listId: string;
    labels: string[];
    assignees: string[];
  }

  function formOf(it: Item): Form {
    return {
      title: it.title,
      body: it.body ?? "",
      dueDate: it.due?.slice(0, 10) ?? "",
      dueTime: it.due && it.due.length > 10 ? it.due.slice(11, 16) : "",
      priority: it.priority,
      listId: it.containerId,
      labels: it.labels.map((l) => l.name),
      assignees: [...it.assignees],
    };
  }

  let form = $state<Form>(formOf(initial));
  let original = $state<Form>(formOf(initial));
  // Reminders are complete in the cache; GitHub bodies are truncated there, so
  // editing waits for the full text.
  let loaded = $state(isReminder);
  let loadError = $state("");
  let saving = $state(false);
  let discardArmed = $state(false);
  let labelOptions = $state<Label[]>([]);
  let assigneeOptions = $state<string[]>([]);
  let labelInput = $state("");
  let assigneeInput = $state("");
  let titleEl = $state<HTMLTextAreaElement>();

  onMount(async () => {
    await tick();
    titleEl?.focus();
    autosize(titleEl);
    if (isReminder) return;
    try {
      const d = await api.itemDetails(initial.id);
      // Keep anything typed while loading; take fresh values for untouched fields.
      const fresh = formOf(d.item);
      for (const k of Object.keys(fresh) as (keyof Form)[]) {
        if (JSON.stringify(form[k]) === JSON.stringify(original[k])) (form as any)[k] = fresh[k];
      }
      original = fresh;
      labelOptions = d.labelOptions;
      assigneeOptions = d.assigneeOptions;
      loaded = true;
      await tick();
      autosize(titleEl);
    } catch (e) {
      loadError = errorText(e);
    }
  });

  // ---- derived state ----------------------------------------------------------

  const lists = $derived(
    (store.snap?.reminderLists ?? []).filter((l) => (l.writable && l.enabled) || l.id === initial.containerId),
  );
  const tagToCategory = $derived(
    new Map(store.categories.flatMap((c) => [[c.name.toLowerCase(), c.id], [c.label.toLowerCase(), c.id]])),
  );
  const due = $derived(form.dueDate ? (form.dueTime ? `${form.dueDate}T${form.dueTime}` : form.dueDate) : "");

  /** Mirrors the backend's rules so the area shown matches where the item will land. */
  const area = $derived.by(() => {
    const tags = isReminder ? [...hashtags(form.title), ...hashtags(form.body)] : form.labels.map((l) => l.toLowerCase());
    for (const t of tags) {
      const id = tagToCategory.get(t);
      if (id) return id;
    }
    return containerDefault;
  });
  const containerDefault = $derived.by(() => {
    if (isReminder) return lists.find((l) => l.id === form.listId)?.defaultCategory ?? null;
    const repo = store.snap?.repos.find((r) => r.id === initial.containerId);
    if (repo?.defaultCategory) return repo.defaultCategory;
    for (const pid of [initial.containerId, ...initial.projectIds]) {
      const p = store.snap?.projects.find((p) => p.id === pid);
      if (p?.defaultCategory) return p.defaultCategory;
    }
    return null;
  });

  const patch = $derived.by((): ItemPatch => {
    const p: ItemPatch = {};
    if (form.title.trim() !== original.title) p.title = form.title.trim();
    if (form.body !== original.body) p.body = form.body;
    if (isReminder) {
      const origDue = original.dueDate ? (original.dueTime ? `${original.dueDate}T${original.dueTime}` : original.dueDate) : "";
      if (due !== origDue) p.due = due;
      if (form.priority !== original.priority) p.priority = form.priority;
      if (form.listId !== original.listId) p.listId = form.listId;
    }
    if (isGitHub) {
      if (!sameSet(form.labels, original.labels)) p.labels = [...form.labels];
      if (!sameSet(form.assignees, original.assignees)) p.assignees = [...form.assignees];
    }
    return p;
  });
  const dirty = $derived(Object.keys(patch).length > 0);
  const canSave = $derived(dirty && loaded && !saving && form.title.trim().length > 0);

  const labelColors = $derived(new Map([...initial.labels, ...labelOptions].map((l) => [l.name.toLowerCase(), l.color])));
  const labelSuggestions = $derived(labelOptions.filter((l) => !form.labels.some((x) => x.toLowerCase() === l.name.toLowerCase())));
  const assigneeSuggestions = $derived(assigneeOptions.filter((a) => !form.assignees.includes(a)));
  const myLogin = $derived(store.snap?.accounts.find((a) => a.id === initial.accountId)?.login);

  // ---- helpers ----------------------------------------------------------------

  function stripCategoryTags(text: string): string {
    return stripTags(text, (t) => tagToCategory.has(t));
  }

  function sameSet(a: string[], b: string[]) {
    return a.length === b.length && a.every((x) => b.some((y) => y.toLowerCase() === x.toLowerCase()));
  }

  function autosize(el?: HTMLTextAreaElement) {
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }

  function setArea(id: string | null) {
    const cat = store.categories.find((c) => c.id === id);
    if (isGitHub) {
      form.labels = form.labels.filter((l) => !tagToCategory.has(l.toLowerCase()));
      if (cat) form.labels.push(cat.label);
      return;
    }
    if (!isReminder) return;
    form.title = stripCategoryTags(form.title);
    form.body = stripCategoryTags(form.body);
    if (!cat) return;
    const mapped = lists.find((l) => l.enabled && l.writable && l.defaultCategory === cat.id);
    if (mapped) form.listId = mapped.id;
    else form.body = form.body ? `${form.body}\n#${cat.label}` : `#${cat.label}`;
  }

  function addLabel(name: string) {
    const n = name.trim();
    if (n && !form.labels.some((l) => l.toLowerCase() === n.toLowerCase())) form.labels.push(n);
    labelInput = "";
  }

  function addAssignee(login: string) {
    const n = login.trim().replace(/^@/, "");
    if (n && !form.assignees.includes(n)) form.assignees.push(n);
    assigneeInput = "";
  }

  // ---- actions ----------------------------------------------------------------

  /** Returns whether the changes were saved. */
  async function save(): Promise<boolean> {
    if (!canSave) return false;
    saving = true;
    try {
      await api.updateItem(initial.id, patch);
      store.toast("Saved", "info");
      await store.refresh();
      close(true);
      return true;
    } catch (e) {
      store.toast(errorText(e));
      return false;
    } finally {
      saving = false;
    }
  }

  /** Completing saves pending edits first, so nothing typed is lost. */
  async function toggleComplete() {
    if (dirty && !(await save())) return;
    close(true);
    store.toggle(initial);
  }

  function close(force = false) {
    if (dirty && !force && !discardArmed) {
      discardArmed = true;
      return;
    }
    store.editing = null;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      // Esc inside a chip input just clears it.
      if (e.target instanceof HTMLInputElement && e.target.value && e.target.closest(".chips")) {
        e.target.value = "";
        return;
      }
      close();
    } else if (e.metaKey && (e.key === "s" || e.key === "Enter")) {
      e.preventDefault();
      save();
    }
  }

  $effect(() => {
    // Any further edit cancels a pending "discard?" prompt.
    void patch;
    discardArmed = false;
  });

  const kindLabel = { reminder: "Reminder", issue: "Issue", pullRequest: "Pull request", draftIssue: "Draft issue" }[
    initial.kind
  ];
  const completeLabel = initial.completed
    ? "Mark as not done"
    : isReminder
      ? "Mark complete"
      : isDraft
        ? "Archive draft"
        : "Close issue";
</script>

<svelte:window {onkeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={() => close()}></div>

<div class="panel" role="dialog" tabindex="-1" aria-label="Edit {kindLabel.toLowerCase()}">
  <header>
    <Icon name={isReminder ? "list" : initial.kind === "pullRequest" ? "pr" : isDraft ? "draft" : "issue"} size={14} />
    <span class="where">
      {kindLabel} · {initial.containerName}{#if initial.number}&nbsp;<span class="faint">#{initial.number}</span>{/if}
    </span>
    <button class="icon-btn" title={isReminder ? "Open in Reminders" : "Open on GitHub"} onclick={() => store.open(initial)}>
      <Icon name={isReminder ? "external" : "github"} />
    </button>
    <button class="icon-btn" title="Close (esc)" onclick={() => close()}><Icon name="x" /></button>
  </header>

  <div class="content">
    <textarea
      bind:this={titleEl}
      class="title selectable"
      rows="1"
      bind:value={form.title}
      oninput={(e) => autosize(e.currentTarget)}
      onkeydown={(e) => {
        if (e.key === "Enter" && !e.metaKey) e.preventDefault();
      }}
      placeholder="Title"
      spellcheck="true"
    ></textarea>

    {#if loadError}
      <p class="error selectable"><Icon name="alert" size={13} /> Couldn't load the latest from GitHub: {loadError}</p>
    {/if}

    <div class="fields">
      <span class="label">Area</span>
      <div class="control">
        {#if isDraft}
          <span class="faint">Drafts can't have labels — set the project's default area in Settings.</span>
        {:else}
          <CategorySelect value={area} onchange={setArea} />
          {#if isReminder && !area && containerDefault === null}
            <!-- nothing to explain -->
          {:else if isReminder && area === containerDefault && containerDefault}
            <span class="faint hint">from the list</span>
          {/if}
        {/if}
      </div>

      {#if isReminder}
        <span class="label">List</span>
        <div class="control">
          <select class="field" bind:value={form.listId}>
            {#each lists as l (l.id)}
              <option value={l.id}>{l.title}{l.sourceTitle ? ` — ${l.sourceTitle}` : ""}</option>
            {/each}
          </select>
        </div>

        <span class="label">Due</span>
        <div class="control wrap">
          <input type="date" class="field" bind:value={form.dueDate} />
          {#if form.dueDate}
            <input type="time" class="field" bind:value={form.dueTime} title="Time (adds an alert)" />
            <button class="btn ghost" onclick={() => ((form.dueDate = ""), (form.dueTime = ""))}>Clear</button>
          {:else}
            <button class="btn ghost" onclick={() => (form.dueDate = todayStr())}>Today</button>
            <button class="btn ghost" onclick={() => (form.dueDate = todayStr(1))}>Tomorrow</button>
          {/if}
        </div>

        <span class="label">Priority</span>
        <div class="control">
          <div class="seg">
            {#each [[0, "None"], [9, "Low"], [5, "Medium"], [1, "High"]] as [v, name] (v)}
              <button
                class:on={form.priority === v || (v === 1 && form.priority > 0 && form.priority < 5) || (v === 9 && form.priority > 5)}
                onclick={() => (form.priority = v as number)}>{name}</button
              >
            {/each}
          </div>
        </div>
      {/if}

      {#if isGitHub}
        <span class="label">Labels</span>
        <div class="control">
          <div class="chips">
            {#each form.labels as l (l)}
              {@const color = labelColors.get(l.toLowerCase())}
              <span class="chip" style={color ? `--label: #${color}` : ""}>
                {l}
                <button aria-label="Remove {l}" onclick={() => (form.labels = form.labels.filter((x) => x !== l))}><Icon name="x" size={10} /></button>
              </span>
            {/each}
            <input
              list="label-options"
              bind:value={labelInput}
              placeholder={loaded ? "Add label…" : "Loading…"}
              disabled={!loaded}
              onkeydown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addLabel(labelInput);
                } else if (e.key === "Backspace" && !labelInput && form.labels.length) {
                  form.labels.pop();
                }
              }}
              onchange={(e) => labelSuggestions.some((l) => l.name === e.currentTarget.value) && addLabel(e.currentTarget.value)}
            />
            <datalist id="label-options">
              {#each labelSuggestions as l (l.name)}<option value={l.name}></option>{/each}
            </datalist>
          </div>
        </div>

        <span class="label">Assignees</span>
        <div class="control">
          <div class="chips">
            {#each form.assignees as a (a)}
              <span class="chip">
                @{a}
                <button aria-label="Unassign {a}" onclick={() => (form.assignees = form.assignees.filter((x) => x !== a))}><Icon name="x" size={10} /></button>
              </span>
            {/each}
            <input
              list="assignee-options"
              bind:value={assigneeInput}
              placeholder={loaded ? "Add assignee…" : "Loading…"}
              disabled={!loaded}
              onkeydown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  addAssignee(assigneeInput);
                } else if (e.key === "Backspace" && !assigneeInput && form.assignees.length) {
                  form.assignees.pop();
                }
              }}
              onchange={(e) => assigneeSuggestions.includes(e.currentTarget.value) && addAssignee(e.currentTarget.value)}
            />
            <datalist id="assignee-options">
              {#each assigneeSuggestions as a (a)}<option value={a}></option>{/each}
            </datalist>
            {#if myLogin && !form.assignees.includes(myLogin) && loaded}
              <button class="btn ghost small" onclick={() => addAssignee(myLogin!)}>Assign me</button>
            {/if}
          </div>
        </div>
      {/if}

      {#if initial.status}
        <span class="label">Status</span>
        <div class="control"><span class="status">{initial.status}</span><span class="faint hint">change it on GitHub</span></div>
      {/if}
    </div>

    <label class="body-label" for="edit-body">
      {isReminder ? "Notes" : "Description"}
      {#if !isReminder}<span class="faint">Markdown</span>{/if}
    </label>
    <textarea
      id="edit-body"
      class="field body selectable"
      bind:value={form.body}
      disabled={!loaded}
      placeholder={loaded ? (isReminder ? "Add notes" : "Add a description") : "Loading the full description…"}
      spellcheck="true"
    ></textarea>
  </div>

  <footer>
    {#if initial.kind !== "pullRequest"}
      <button class="btn" onclick={toggleComplete} disabled={saving || (dirty && !canSave)}>
        <Icon name="check" size={14} />
        {completeLabel}
      </button>
    {/if}
    <span class="spacer"></span>
    {#if discardArmed}
      <span class="warn">Unsaved changes — esc again to discard</span>
    {:else if dirty}
      <span class="faint">⌘S to save</span>
    {/if}
    <button class="btn" onclick={() => close(true)}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={!canSave}>{saving ? "Saving…" : "Save"}</button>
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(0, 0, 0, 0.12);
    animation: fade 0.15s ease-out;
  }
  @media (prefers-color-scheme: dark) {
    .scrim {
      background: rgba(0, 0, 0, 0.35);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .panel {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 41;
    width: min(460px, 100vw - 120px);
    display: flex;
    flex-direction: column;
    background: var(--bg);
    border-left: 1px solid var(--border);
    box-shadow: var(--shadow);
    animation: slide 0.18s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  @keyframes slide {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 46px;
    padding: 0 10px 0 18px;
    border-bottom: 1px solid var(--border);
    color: var(--text-2);
    font-size: 12px;
  }
  .where {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .content {
    flex: 1;
    overflow-y: auto;
    padding: 16px 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .title {
    width: 100%;
    border: none;
    background: none;
    outline: none;
    resize: none;
    padding: 0;
    font-size: 18px;
    font-weight: 600;
    line-height: 1.35;
    overflow: hidden;
  }
  .error {
    display: flex;
    gap: 6px;
    margin: 0;
    color: var(--danger);
    font-size: 12px;
  }
  .fields {
    display: grid;
    grid-template-columns: 78px 1fr;
    align-items: center;
    gap: 10px 12px;
  }
  .label,
  .body-label {
    color: var(--text-2);
    font-size: 12px;
  }
  .control {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .control.wrap {
    flex-wrap: wrap;
  }
  .control select {
    min-width: 200px;
    height: 26px;
  }
  .control .field {
    height: 26px;
  }
  .hint {
    font-size: 11.5px;
  }
  .seg {
    display: flex;
    padding: 2px;
    border-radius: 7px;
    background: var(--bg-input);
  }
  .seg button {
    height: 22px;
    padding: 0 10px;
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
  .chips {
    flex: 1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 5px;
    min-height: 28px;
    padding: 3px 6px;
    border-radius: 6px;
    border: 1px solid var(--border);
    background: var(--bg-input);
  }
  .chips:focus-within {
    border-color: color-mix(in srgb, var(--accent) 70%, transparent);
  }
  .chips input {
    flex: 1;
    min-width: 90px;
    border: none;
    background: none;
    outline: none;
    height: 20px;
  }
  .chip {
    --label: var(--text-3);
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 0 4px 0 8px;
    border-radius: 10px;
    line-height: 18px;
    font-size: 12px;
    border: 1px solid color-mix(in srgb, var(--label) 60%, transparent);
    background: color-mix(in srgb, var(--label) 18%, transparent);
    color: color-mix(in srgb, var(--label) 55%, var(--text));
  }
  .chip button {
    display: grid;
    place-items: center;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    color: inherit;
    opacity: 0.7;
  }
  .chip button:hover {
    opacity: 1;
    background: var(--bg-active);
  }
  .small {
    height: 20px;
    padding: 0 6px;
    font-size: 11.5px;
  }
  .status {
    padding: 0 6px;
    border-radius: 4px;
    background: var(--bg-active);
    font-size: 12px;
  }
  .body-label {
    display: flex;
    justify-content: space-between;
    margin-bottom: -8px;
  }
  .body {
    flex: 1;
    min-height: 180px;
    resize: none;
    line-height: 1.5;
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
    font-size: 12px;
  }
  .spacer {
    flex: 1;
  }
  .warn {
    color: var(--warn);
  }
</style>
