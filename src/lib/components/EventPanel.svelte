<script lang="ts">
  import { onMount, tick } from "svelte";
  import type { CalendarEvent, EventPatch } from "$lib/api";
  import { dueMinutes, formatTime, parseYmd, ymd } from "$lib/dates";
  import { store } from "$lib/store.svelte";
  import { hashtags, stripTags } from "$lib/tags";
  import CategorySelect from "./CategorySelect.svelte";
  import Icon from "./Icon.svelte";

  let { event }: { event: CalendarEvent } = $props();

  // Keyed by event id in the page, so reading the prop once is intended.
  // svelte-ignore state_referenced_locally
  const initial = event;

  interface Form {
    title: string;
    calendarId: string;
    allDay: boolean;
    startDate: string;
    startTime: string;
    endDate: string;
    endTime: string;
    location: string;
    notes: string;
  }

  /** `HH:MM` ↔ minutes after midnight. */
  const hm = (m: number) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
  const minutesOf = (t: string) => {
    const [h, m] = t.split(":").map(Number);
    return h * 60 + (m || 0);
  };
  const time = (s: string) => {
    const m = dueMinutes(s);
    return m === null ? "" : hm(m);
  };
  const original: Form = {
    title: initial.title,
    calendarId: initial.calendarId,
    allDay: initial.allDay,
    startDate: initial.start.slice(0, 10),
    startTime: time(initial.start) || "09:00",
    endDate: initial.end.slice(0, 10),
    endTime: time(initial.end) || "10:00",
    location: initial.location ?? "",
    notes: initial.notes ?? "",
  };
  let form = $state<Form>({ ...original });
  let saving = $state(false);
  let discardArmed = $state(false);
  let titleEl = $state<HTMLTextAreaElement>();

  const readOnly = !initial.writable;
  const calendars = $derived(
    (store.snap?.eventCalendars ?? []).filter((c) => (c.writable && c.enabled) || c.id === initial.calendarId),
  );
  const tagToCategory = $derived(
    new Map(store.categories.flatMap((c) => [[c.name.toLowerCase(), c.id], [c.label.toLowerCase(), c.id]])),
  );

  const start = $derived(form.allDay ? form.startDate : `${form.startDate}T${form.startTime}`);
  const end = $derived(form.allDay ? form.endDate : `${form.endDate}T${form.endTime}`);
  const invalid = $derived(
    !form.title.trim() || !form.startDate || !form.endDate || (form.allDay ? end < start : end <= start),
  );

  const patch = $derived.by((): EventPatch => {
    const p: EventPatch = {};
    if (form.title.trim() !== original.title) p.title = form.title.trim();
    if (form.calendarId !== original.calendarId) p.calendarId = form.calendarId;
    if (form.location !== original.location) p.location = form.location;
    if (form.notes !== original.notes) p.notes = form.notes;
    if (start !== initial.start || end !== initial.end || form.allDay !== original.allDay) {
      p.start = start;
      p.end = end;
      p.allDay = form.allDay;
    }
    return p;
  });
  const dirty = $derived(Object.keys(patch).length > 0);
  const canSave = $derived(dirty && !invalid && !saving && !readOnly);

  /** Same rules as the backend: `#tags` beat the calendar's default area. */
  const area = $derived.by(() => {
    for (const t of [...hashtags(form.title), ...hashtags(form.notes)]) {
      const id = tagToCategory.get(t);
      if (id) return id;
    }
    return calendars.find((c) => c.id === form.calendarId)?.defaultCategory ?? null;
  });

  function setArea(id: string | null) {
    const cat = store.categories.find((c) => c.id === id);
    const strip = (t: string) => stripTags(t, (tag) => tagToCategory.has(tag));
    form.title = strip(form.title);
    form.notes = strip(form.notes);
    if (!cat) return;
    const mapped = calendars.find((c) => c.writable && c.defaultCategory === cat.id);
    if (mapped) form.calendarId = mapped.id;
    else form.notes = form.notes ? `${form.notes}\n#${cat.label}` : `#${cat.label}`;
  }

  // Keep the end after the start when the start moves. Seeded with the
  // starting value on purpose; the effect tracks later changes.
  // svelte-ignore state_referenced_locally
  let prevStart = start;
  $effect(() => {
    const s = start;
    if (s === prevStart) return;
    const was = prevStart;
    prevStart = s;
    if (!was || !s) return;
    // Shift the end by the same amount, keeping the length.
    const delta = parseYmd(s).getTime() - parseYmd(was).getTime();
    if (delta) form.endDate = ymd(new Date(parseYmd(form.endDate).getTime() + delta));
    if (!form.allDay && form.startDate === form.endDate && form.endTime <= form.startTime) {
      const m = minutesOf(form.startTime) + 60;
      form.endTime = m >= 1440 ? "23:59" : hm(m);
    }
  });

  async function save(): Promise<boolean> {
    if (!canSave) return false;
    saving = true;
    const ok = await store.updateEvent(initial, patch);
    saving = false;
    if (ok) {
      store.toast("Saved", "info");
      close(true);
    }
    return ok;
  }

  function close(force = false) {
    if (dirty && !force && !discardArmed) {
      discardArmed = true;
      return;
    }
    store.editingEvent = null;
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      close();
    } else if (e.metaKey && (e.key === "s" || e.key === "Enter")) {
      e.preventDefault();
      save();
    }
  }

  $effect(() => {
    void patch;
    discardArmed = false;
  });

  onMount(async () => {
    await tick();
    if (!readOnly) titleEl?.focus();
  });

  const when = $derived.by(() => {
    const d = parseYmd(initial.start).toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });
    if (initial.allDay) {
      return initial.end === initial.start
        ? `${d}, all day`
        : `${d} – ${parseYmd(initial.end).toLocaleDateString(undefined, { month: "long", day: "numeric" })}`;
    }
    return `${d}, ${formatTime(dueMinutes(initial.start) ?? 0)} – ${formatTime(dueMinutes(initial.end) ?? 0)}`;
  });
</script>

<svelte:window {onkeydown} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="scrim" onclick={() => close()}></div>

<div class="panel" role="dialog" tabindex="-1" aria-label="Edit event" style:--c={initial.color ?? "var(--accent)"}>
  <header>
    <span class="swatch"></span>
    <span class="where">Event · {initial.calendarName}</span>
    <button class="icon-btn" title="Open in Calendar" onclick={() => store.openEvent(initial)}><Icon name="external" /></button>
    <button class="icon-btn" title="Close (esc)" onclick={() => close()}><Icon name="x" /></button>
  </header>

  <div class="content">
    <textarea
      bind:this={titleEl}
      class="title selectable"
      rows="1"
      bind:value={form.title}
      readonly={readOnly}
      onkeydown={(e) => e.key === "Enter" && !e.metaKey && e.preventDefault()}
      placeholder="Title"
    ></textarea>
    <p class="when faint">{when}</p>

    {#if readOnly}
      <p class="note"><Icon name="alert" size={13} /> This calendar is read-only.</p>
    {:else if initial.recurring}
      <p class="note"><Icon name="sync" size={13} /> Repeating event — changes apply to this occurrence only.</p>
    {/if}

    <div class="fields">
      <span class="label">Area</span>
      <div class="control">
        <CategorySelect value={area} onchange={setArea} />
      </div>

      <span class="label">Calendar</span>
      <div class="control">
        <select class="field" bind:value={form.calendarId} disabled={readOnly || initial.recurring}>
          {#each calendars as c (c.id)}
            <option value={c.id}>{c.title}{c.sourceTitle ? ` — ${c.sourceTitle}` : ""}</option>
          {/each}
        </select>
      </div>

      <span class="label">All day</span>
      <div class="control">
        <input type="checkbox" bind:checked={form.allDay} disabled={readOnly} />
      </div>

      <span class="label">Starts</span>
      <div class="control">
        <input type="date" class="field" bind:value={form.startDate} disabled={readOnly} />
        {#if !form.allDay}<input type="time" class="field" bind:value={form.startTime} disabled={readOnly} />{/if}
      </div>

      <span class="label">Ends</span>
      <div class="control">
        <input type="date" class="field" bind:value={form.endDate} min={form.startDate} disabled={readOnly} />
        {#if !form.allDay}<input type="time" class="field" bind:value={form.endTime} disabled={readOnly} />{/if}
        {#if invalid && form.title.trim()}<span class="error">Ends before it starts</span>{/if}
      </div>

      <span class="label">Location</span>
      <div class="control">
        <input class="field grow" bind:value={form.location} placeholder="Add a location" readonly={readOnly} />
      </div>
    </div>

    <label class="body-label" for="event-notes">Notes</label>
    <textarea id="event-notes" class="field body selectable" bind:value={form.notes} placeholder="Add notes" readonly={readOnly}></textarea>
  </div>

  <footer>
    <span class="spacer"></span>
    {#if discardArmed}
      <span class="warn">Unsaved changes — esc again to discard</span>
    {:else if dirty}
      <span class="faint">⌘S to save</span>
    {/if}
    <button class="btn" onclick={() => close(true)}>{readOnly ? "Close" : "Cancel"}</button>
    {#if !readOnly}
      <button class="btn primary" onclick={save} disabled={!canSave}>{saving ? "Saving…" : "Save"}</button>
    {/if}
  </footer>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: rgba(0, 0, 0, 0.12);
  }
  @media (prefers-color-scheme: dark) {
    .scrim {
      background: rgba(0, 0, 0, 0.35);
    }
  }
  .panel {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    z-index: 41;
    width: min(440px, 100vw - 120px);
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
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    background: var(--c);
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
    gap: 12px;
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
    field-sizing: content;
  }
  .when {
    margin: -8px 0 0;
    font-size: 12px;
  }
  .note {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 6px 10px;
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text-2);
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
  .control .field {
    height: 26px;
  }
  .control select {
    min-width: 200px;
  }
  .grow {
    flex: 1;
  }
  .error {
    color: var(--danger);
    font-size: 11.5px;
  }
  .body-label {
    margin-bottom: -6px;
  }
  .body {
    flex: 1;
    min-height: 140px;
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
