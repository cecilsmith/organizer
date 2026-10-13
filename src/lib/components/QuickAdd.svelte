<script lang="ts">
  import { api, errorText, type NewEvent, type NewItem } from "$lib/api";
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
  // Events only
  let endTime = $state("");
  let allDay = $state(false);
  let location = $state("");
  let busy = $state(false);
  let expanded = $state(false);

  const snap = $derived(store.snap);
  const category = $derived(store.categories.some((c) => c.id === store.tab) ? store.tab : null);
  const categoryName = $derived(store.categories.find((c) => c.id === category)?.name);
  const lists = $derived(snap?.reminderLists.filter((l) => l.enabled && l.writable) ?? []);
  const repos = $derived(snap?.repos ?? []);
  const githubRepos = $derived(repos.filter((r) => r.provider !== "gitlab"));
  const gitlabRepos = $derived(repos.filter((r) => r.provider === "gitlab"));
  const calendars = $derived(
    snap?.calendarAccess === "granted" ? snap.eventCalendars.filter((c) => c.enabled && c.writable) : [],
  );
  const hasTargets = $derived(lists.length + repos.length + calendars.length > 0);

  // Remember the last target per tab; otherwise prefer a source mapped to this area.
  const targetKey = $derived(`organizer.target.${store.tab}`);
  let chosen = $state<Record<string, string>>({});

  function isValid(t: string | null | undefined): t is string {
    if (!t) return false;
    const [kind, id] = [t.slice(0, t.indexOf(":")), t.slice(t.indexOf(":") + 1)];
    if (kind === "rem") return lists.some((l) => l.id === id);
    if (kind === "cal") return calendars.some((c) => c.id === id);
    return repos.some((r) => r.id === id);
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
    if (calendars[0]) return `cal:${calendars[0].id}`;
    return "";
  });

  const isReminder = $derived(target.startsWith("rem:"));
  const isEvent = $derived(target.startsWith("cal:"));
  const targetId = $derived(target.slice(target.indexOf(":") + 1));
  const repo = $derived(repos.find((r) => r.id === targetId));
  const projects = $derived(snap?.projects.filter((p) => repo && p.accountId === repo.accountId) ?? []);

  function pick(value: string) {
    chosen[store.tab] = value;
    try {
      localStorage.setItem(targetKey, value);
    } catch {}
  }

  // The calendar asks for a prefilled date (double-click on a day or time slot).
  $effect(() => {
    const preset = store.quickAddDue;
    if (preset === null) return;
    store.quickAddDue = null;
    due = preset.slice(0, 10);
    time = preset.length > 10 ? preset.slice(11, 16) : "";
    expanded = true;
    if (isEvent) {
      // A day becomes an all-day event; a time slot becomes a one-hour event.
      allDay = !time;
      endTime = time ? plusHour(time) : "";
    } else if (!isReminder) {
      // GitHub issues have no dates; switch to a reminder list.
      if (lists[0]) pick(`rem:${lists[0].id}`);
      else if (calendars[0]) pick(`cal:${calendars[0].id}`);
    }
    inputEl?.focus();
  });

  function reset() {
    title = "";
    notes = "";
    due = "";
    time = "";
    endTime = "";
    allDay = false;
    location = "";
    priority = 0;
    expanded = false;
  }

  function plusHour(t: string) {
    const [h, m] = t.split(":").map(Number);
    const total = Math.min(h * 60 + m + 60, 23 * 60 + 59);
    return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
  }

  /** The next whole hour, for a new event with no time picked yet. */
  function nextHour() {
    const h = Math.min(new Date().getHours() + 1, 23);
    return `${String(h).padStart(2, "0")}:00`;
  }

  async function submitEvent(t: string) {
    const day = due || todayStr();
    const start = allDay ? day : `${day}T${time || nextHour()}`;
    const end = allDay ? day : `${day}T${endTime || plusHour(time || nextHour())}`;
    const req: NewEvent = {
      calendarId: targetId,
      title: t,
      start,
      end,
      allDay,
      location: location || null,
      notes: notes || null,
      category,
    };
    const ev = await api.createEvent(req);
    // Undo deletes the event; redo creates it again (with a new id).
    let current = ev;
    const entry = store.record(
      `add “${ev.title}”`,
      () => api.deleteEvent(current.id, current),
      async () => {
        current = await api.createEvent(req);
      },
    );
    store.toast(`Added “${ev.title}” to ${ev.calendarName}`, "info", { label: "Undo", run: () => store.undo(entry) });
  }

  async function submit() {
    const t = title.trim();
    if (!t || busy || !target) return;
    busy = true;
    if (isEvent) {
      try {
        await submitEvent(t);
        reset();
        await store.refresh();
      } catch (e) {
        store.toast(errorText(e));
      } finally {
        busy = false;
      }
      return;
    }
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
        placeholder={isEvent ? "New event…" : categoryName ? `New ${categoryName.toLowerCase()} to-do…` : "New to-do…"}
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
        {#if calendars.length}
          <optgroup label="Calendar event in…">
            {#each calendars as c (c.id)}
              <option value={`cal:${c.id}`}>{c.title}{c.sourceTitle ? ` (${c.sourceTitle})` : ""}</option>
            {/each}
          </optgroup>
        {/if}
        {#if githubRepos.length}
          <optgroup label="GitHub issue in…">
            {#each githubRepos as r (r.id)}
              <option value={`repo:${r.id}`}>{r.owner}/{r.name}</option>
            {/each}
          </optgroup>
        {/if}
        {#if gitlabRepos.length}
          <optgroup label="GitLab issue in…">
            {#each gitlabRepos as r (r.id)}
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
          placeholder={isReminder || isEvent ? "Notes" : "Issue description (Markdown)"}
          onfocus={() => (expanded = true)}
        ></textarea>
        <div class="opts">
          {#if isEvent}
            <input type="date" class="field" value={due || todayStr()} oninput={(e) => (due = e.currentTarget.value)} />
            <label class="check"><input type="checkbox" bind:checked={allDay} /> All day</label>
            {#if !allDay}
              <input
                type="time"
                class="field"
                value={time || nextHour()}
                oninput={(e) => {
                  time = e.currentTarget.value;
                  endTime = plusHour(time);
                }}
                title="Starts"
              />
              <span class="faint">to</span>
              <input
                type="time"
                class="field"
                value={endTime || plusHour(time || nextHour())}
                oninput={(e) => (endTime = e.currentTarget.value)}
                title="Ends"
              />
            {/if}
            <input class="field" bind:value={location} placeholder="Location" />
          {:else if isReminder}
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
    <span class="muted">Connect Apple Reminders, your calendars, or a GitHub or GitLab project to start adding things.</span>
    <button class="btn" onclick={() => store.openSettings("reminders")}>Connect Reminders</button>
    <button class="btn" onclick={() => store.openSettings("accounts")}><Icon name="github" size={14} /> Link GitHub or GitLab</button>
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
