<script lang="ts">
  import { store } from "$lib/store.svelte";

  let {
    value,
    onchange,
    none = "None (Inbox)",
  }: { value: string | null; onchange: (v: string | null) => void; none?: string } = $props();

  const color = $derived(store.categories.find((c) => c.id === value)?.color);
</script>

<div class="wrap">
  <span class="dot" style:background={color ?? "transparent"} style:border={color ? "none" : "1px solid var(--border-strong)"}></span>
  <select class="field" value={value ?? ""} onchange={(e) => onchange(e.currentTarget.value || null)}>
    <option value="">{none}</option>
    {#each store.categories as c (c.id)}
      <option value={c.id}>{c.name}</option>
    {/each}
  </select>
</div>

<style>
  .wrap {
    position: relative;
    display: inline-flex;
    align-items: center;
  }
  .dot {
    position: absolute;
    left: 9px;
    z-index: 1;
    pointer-events: none;
  }
  select.field {
    padding-left: 25px;
    height: 26px;
    min-width: 130px;
  }
</style>
