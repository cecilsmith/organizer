<script lang="ts">
  import { api, errorText, type Candidate } from "$lib/api";
  import { store } from "$lib/store.svelte";
  import CategorySelect from "./CategorySelect.svelte";

  let { kind }: { kind: "repo" | "project" } = $props();

  // GitHub Projects only exist on GitHub.
  const accounts = $derived((store.snap?.accounts ?? []).filter((a) => kind === "repo" || a.provider === "github"));
  let accountId = $state("");
  let candidates = $state<Candidate[] | null>(null);
  let loading = $state(false);
  let error = $state("");
  let filter = $state("");
  let manual = $state("");
  let category = $state<string | null>(null);
  let adding = $state<string | null>(null);

  const account = $derived(accounts.find((a) => a.id === accountId) ?? accounts[0]);
  // A different account means a different list to browse.
  $effect(() => {
    void account?.id;
    candidates = null;
    error = "";
  });
  const tracked = $derived(
    new Set(kind === "repo" ? (store.snap?.repos ?? []).map((r) => r.id) : (store.snap?.projects ?? []).map((p) => p.id)),
  );
  const shown = $derived(
    (candidates ?? [])
      .filter((c) => !tracked.has(c.id))
      .filter((c) => `${c.owner}/${c.name}`.toLowerCase().includes(filter.trim().toLowerCase()))
      .slice(0, 60),
  );

  async function browse() {
    if (!account) return;
    loading = true;
    error = "";
    try {
      candidates =
        kind === "repo" ? await api.repoCandidates(account.id) : await api.projectCandidates(account.id);
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }

  async function add(c: Candidate) {
    if (!account) return;
    adding = c.id;
    try {
      if (kind === "repo") await api.addRepo(account.id, c, category);
      else await api.addProject(account.id, c, category);
    } catch (e) {
      store.toast(errorText(e));
    } finally {
      adding = null;
    }
  }

  async function addManual() {
    if (!account || !manual.trim()) return;
    error = "";
    try {
      const c =
        kind === "repo"
          ? await api.lookupRepo(account.id, manual)
          : await api.lookupProject(account.id, manual);
      await add(c);
      manual = "";
    } catch (e) {
      error = errorText(e);
    }
  }
</script>

{#if !accounts.length}
  <p class="muted">Link {kind === "repo" ? "a GitHub or GitLab" : "a GitHub"} account first.</p>
{:else}
  <div class="picker">
    <div class="row">
      {#if accounts.length > 1}
        <select class="field" bind:value={accountId}>
          {#each accounts as a (a.id)}<option value={a.id}>{a.id}</option>{/each}
        </select>
      {/if}
      <span class="muted">Default area</span>
      <CategorySelect value={category} onchange={(v) => (category = v)} />
      <button class="btn" onclick={browse} disabled={loading}>
        {loading
          ? "Loading…"
          : candidates
            ? "Reload"
            : kind === "repo"
              ? account?.provider === "gitlab"
                ? "Browse my GitLab projects"
                : "Browse my repos"
              : "Browse my projects"}
      </button>
    </div>
    <div class="row">
      <input
        class="field grow"
        bind:value={manual}
        placeholder={kind === "project"
          ? "owner/number or project URL"
          : account?.provider === "gitlab"
            ? "group/project or GitLab URL"
            : "owner/repo or GitHub URL"}
        onkeydown={(e) => e.key === "Enter" && addManual()}
      />
      <button class="btn" onclick={addManual} disabled={!manual.trim()}>Add</button>
    </div>
    {#if error}<p class="error selectable">{error}</p>{/if}
    {#if candidates}
      <input class="field" bind:value={filter} placeholder="Filter…" />
      <div class="list">
        {#each shown as c (c.id)}
          <div class="cand">
            <div class="grow">
              <div>{c.owner}/{c.name}{#if c.number}<span class="faint"> #{c.number}</span>{/if}</div>
              {#if c.description}<div class="faint desc">{c.description}</div>{/if}
            </div>
            <button class="btn" onclick={() => add(c)} disabled={adding === c.id}>
              {adding === c.id ? "Adding…" : "Track"}
            </button>
          </div>
        {:else}
          <p class="faint">Nothing {filter ? "matches" : "left to add"}.</p>
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .picker {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: 8px;
    background: var(--bg-input);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .list {
    max-height: 260px;
    overflow-y: auto;
    border-radius: 6px;
    background: var(--bg);
    border: 1px solid var(--border);
  }
  .cand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .cand:last-child {
    border-bottom: none;
  }
  .desc {
    font-size: 11.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .error {
    color: var(--danger);
    margin: 0;
  }
  .list p {
    padding: 8px 10px;
    margin: 0;
  }
</style>
