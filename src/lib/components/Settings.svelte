<script lang="ts">
  import { api, errorText, type Category } from "$lib/api";
  import { store, relativeTime } from "$lib/store.svelte";
  import CategorySelect from "./CategorySelect.svelte";
  import Icon from "./Icon.svelte";
  import SourcePicker from "./SourcePicker.svelte";

  const sections = [
    { id: "accounts", name: "GitHub accounts", icon: "github" },
    { id: "repos", name: "Repositories", icon: "issue" },
    { id: "projects", name: "Projects", icon: "stack" },
    { id: "reminders", name: "Apple Reminders", icon: "list" },
    { id: "categories", name: "Areas", icon: "tag" },
    { id: "status", name: "Sync status", icon: "sync" },
  ];

  const snap = $derived(store.snap);

  // Destructive actions take two clicks: the first arms the button, the second runs it.
  let armed = $state<string | null>(null);
  let armTimer: ReturnType<typeof setTimeout> | undefined;
  function confirmed(key: string) {
    if (armed === key) {
      armed = null;
      return true;
    }
    armed = key;
    clearTimeout(armTimer);
    armTimer = setTimeout(() => (armed = null), 3000);
    return false;
  }

  // ---- accounts
  let token = $state("");
  let host = $state("github.com");
  let linking = $state(false);
  let importReport = $state<string[]>([]);
  const tokenUrl =
    "https://github.com/settings/tokens/new?scopes=repo,project&description=Organizer%20app";

  async function link() {
    linking = true;
    try {
      const a = await api.addAccount(token, host);
      token = "";
      store.toast(`Linked ${a.id}`, "info");
    } catch (e) {
      store.toast(errorText(e));
    } finally {
      linking = false;
    }
  }

  async function importGh() {
    linking = true;
    importReport = [];
    try {
      importReport = await api.importGhAccounts();
    } catch (e) {
      importReport = [errorText(e)];
    } finally {
      linking = false;
    }
  }

  async function unlink(id: string) {
    if (!confirmed(`acct:${id}`)) return;
    await api.removeAccount(id).catch((e) => store.toast(errorText(e)));
  }

  // ---- reminders
  let requesting = $state(false);
  async function requestAccess() {
    requesting = true;
    try {
      const status = await api.requestRemindersAccess();
      if (status !== "granted") store.toast("Reminders access wasn't granted.");
    } catch (e) {
      store.toast(errorText(e));
    } finally {
      requesting = false;
    }
  }
  const privacyUrl = "x-apple.systempreferences:com.apple.preference.security?Privacy_Reminders";

  // ---- categories
  let draft = $state<Category>({ id: "", name: "", color: "#e5484d", label: "", position: 0 });
  async function saveCategory(c: Category) {
    try {
      await api.upsertCategory(c);
    } catch (e) {
      store.toast(errorText(e));
    }
  }
  async function addCategory() {
    try {
      await api.upsertCategory(draft);
      draft = { id: "", name: "", color: "#e5484d", label: "", position: 0 };
    } catch (e) {
      store.toast(errorText(e));
    }
  }
  async function removeCategory(c: Category) {
    if (!confirmed(`cat:${c.id}`)) return;
    if (store.tab === c.id) store.tab = "all";
    await api.deleteCategory(c.id).catch((e) => store.toast(errorText(e)));
  }

  function setDefault(id: string, v: string | null) {
    api.setDefaultCategory(id, v).catch((e) => store.toast(errorText(e)));
  }

  function removeSource(id: string) {
    if (!confirmed(`src:${id}`)) return;
    api.removeSource(id).catch((e) => store.toast(errorText(e)));
  }

  function close() {
    store.settingsOpen = false;
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="backdrop" onclick={close}>
  <div class="sheet" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Settings" tabindex="-1">
    <nav>
      <h2>Settings</h2>
      {#each sections as s (s.id)}
        <button class:active={store.settingsSection === s.id} onclick={() => (store.settingsSection = s.id)}>
          <Icon name={s.icon} />
          {s.name}
          {#if s.id === "status" && store.sourceErrors.length}<span class="badge">{store.sourceErrors.length}</span>{/if}
        </button>
      {/each}
    </nav>

    <section>
      <button class="icon-btn close" onclick={close} title="Close (esc)"><Icon name="x" /></button>

      {#if store.settingsSection === "accounts"}
        <h3>GitHub accounts</h3>
        <p class="muted">
          Link as many accounts as you like — personal, work, school, or GitHub Enterprise. Tokens are stored in your
          macOS Keychain.
        </p>
        {#each snap?.accounts ?? [] as a (a.id)}
          <div class="card account">
            {#if a.avatarUrl}<img src={a.avatarUrl} alt="" />{/if}
            <div class="grow">
              <div><strong>{a.name ?? a.login}</strong> <span class="faint">{a.login}@{a.host}</span></div>
              {#if a.scopes.length && !a.scopes.some((s) => s.includes("project"))}
                <div class="warn"><Icon name="alert" size={12} /> Missing the <code>project</code> scope — Projects won't load.</div>
              {/if}
            </div>
            <button class="btn ghost danger" onclick={() => unlink(a.id)}>{armed === `acct:${a.id}` ? "Click to confirm" : "Unlink"}</button>
          </div>
        {/each}

        <h4>Import from the GitHub CLI</h4>
        <p class="muted">Uses every account you're logged into with <code>gh auth login</code>.</p>
        <button class="btn" onclick={importGh} disabled={linking}><Icon name="github" size={14} /> Import gh accounts</button>
        {#if importReport.length}
          <ul class="report selectable">{#each importReport as line}<li>{line}</li>{/each}</ul>
        {/if}

        <h4>Or paste a personal access token</h4>
        <p class="muted">
          Needs <code>repo</code> and <code>project</code> scopes.
          <button class="link" onclick={() => api.openUrl(tokenUrl)}>Create one on GitHub →</button>
        </p>
        <div class="row">
          <input class="field" style="width: 150px" bind:value={host} placeholder="github.com" title="Host (change for GitHub Enterprise)" />
          <input class="field grow" type="password" bind:value={token} placeholder="ghp_… or github_pat_…" onkeydown={(e) => e.key === "Enter" && link()} />
          <button class="btn primary" onclick={link} disabled={!token.trim() || linking}>{linking ? "Linking…" : "Link"}</button>
        </div>
      {:else if store.settingsSection === "repos"}
        <h3>Repositories</h3>
        <p class="muted">
          Open issues from these repos appear in your lists. Issues labeled with an area's label (like
          <code>school</code>) go to that area; others use the repo's default area.
        </p>
        {#each snap?.repos ?? [] as r (r.id)}
          <div class="card">
            <Icon name="issue" />
            <div class="grow">{r.owner}/{r.name} <span class="faint">via {r.accountId}</span></div>
            <CategorySelect value={r.defaultCategory} onchange={(v) => setDefault(r.id, v)} />
            <button class="icon-btn" class:armed={armed === `src:${r.id}`} title={armed === `src:${r.id}` ? "Click again to stop tracking" : "Stop tracking (nothing changes on GitHub)"} onclick={() => removeSource(r.id)}><Icon name="trash" /></button>
          </div>
        {/each}
        <h4>Track a repository</h4>
        <SourcePicker kind="repo" />
      {:else if store.settingsSection === "projects"}
        <h3>Projects</h3>
        <p class="muted">
          Items from GitHub Projects (issues, pull requests and drafts) appear with their Status. Requires the
          <code>project</code> token scope.
        </p>
        {#each snap?.projects ?? [] as p (p.id)}
          <div class="card">
            <Icon name="stack" />
            <div class="grow">{p.title} <span class="faint">{p.owner} #{p.number}</span></div>
            <CategorySelect value={p.defaultCategory} onchange={(v) => setDefault(p.id, v)} />
            <button class="icon-btn" class:armed={armed === `src:${p.id}`} title={armed === `src:${p.id}` ? "Click again to stop tracking" : "Stop tracking (nothing changes on GitHub)"} onclick={() => removeSource(p.id)}><Icon name="trash" /></button>
          </div>
        {/each}
        <h4>Track a project</h4>
        <SourcePicker kind="project" />
      {:else if store.settingsSection === "reminders"}
        <h3>Apple Reminders</h3>
        {#if snap?.remindersAccess === "granted"}
          <p class="muted">
            Give each list a default area. A <code>#hashtag</code> matching an area's label in a reminder's title or
            notes overrides the list default. Changes you make in the Reminders app show up here instantly.
          </p>
          {#each snap.reminderLists as l (l.id)}
            <div class="card" class:off={!l.enabled}>
              <input type="checkbox" checked={l.enabled} title="Show this list" onchange={(e) => api.setListEnabled(l.id, e.currentTarget.checked)} />
              <span class="dot" style:background={l.color ?? "var(--text-3)"}></span>
              <div class="grow">
                {l.title}
                {#if l.sourceTitle}<span class="faint">{l.sourceTitle}</span>{/if}
                {#if !l.writable}<span class="faint">· read-only</span>{/if}
              </div>
              <CategorySelect value={l.defaultCategory} onchange={(v) => setDefault(l.id, v)} />
            </div>
          {:else}
            <p class="faint">No Reminders lists found.</p>
          {/each}
        {:else if snap?.remindersAccess === "denied" || snap?.remindersAccess === "restricted" || snap?.remindersAccess === "writeOnly"}
          <p class="muted">Organizer doesn't have full access to Reminders. Turn it on in System Settings → Privacy & Security → Reminders, then come back.</p>
          <button class="btn" onclick={() => api.openUrl(privacyUrl)}>Open Privacy Settings</button>
        {:else if snap?.remindersAccess === "unsupported"}
          <p class="muted">Apple Reminders is only available on macOS.</p>
        {:else}
          <p class="muted">Organizer reads and writes Reminders directly through EventKit. macOS will ask you to allow it once.</p>
          <button class="btn primary" onclick={requestAccess} disabled={requesting}>{requesting ? "Waiting for permission…" : "Connect Apple Reminders"}</button>
        {/if}
      {:else if store.settingsSection === "categories"}
        <h3>Areas</h3>
        <p class="muted">
          Each area is a tab. An item lands in an area when it carries the area's <strong>label</strong> — a GitHub label,
          or a <code>#label</code> hashtag in a reminder — or when its list/repo/project defaults to it.
        </p>
        <div class="cats">
          <div class="cat head"><span></span><span>Name</span><span>Label</span><span></span></div>
          {#each store.categories as c (c.id)}
            <div class="cat">
              <input type="color" value={c.color} onchange={(e) => saveCategory({ ...c, color: e.currentTarget.value })} />
              <input class="field" value={c.name} onchange={(e) => saveCategory({ ...c, name: e.currentTarget.value })} />
              <input class="field mono" value={c.label} onchange={(e) => saveCategory({ ...c, label: e.currentTarget.value.trim() })} />
              <button class="icon-btn" class:armed={armed === `cat:${c.id}`} title={armed === `cat:${c.id}` ? "Click again to delete" : "Delete area (items keep their labels)"} onclick={() => removeCategory(c)}><Icon name="trash" /></button>
            </div>
          {/each}
          <div class="cat">
            <input type="color" bind:value={draft.color} />
            <input class="field" bind:value={draft.name} placeholder="New area…" onkeydown={(e) => e.key === "Enter" && addCategory()} />
            <input class="field mono" bind:value={draft.label} placeholder="label (optional)" onkeydown={(e) => e.key === "Enter" && addCategory()} />
            <button class="icon-btn" title="Add area" onclick={addCategory} disabled={!draft.name.trim()}><Icon name="plus" /></button>
          </div>
        </div>
      {:else if store.settingsSection === "status"}
        <h3>Sync status</h3>
        <p class="muted">GitHub refreshes every 2 minutes and whenever you come back to the window. Reminders update live.</p>
        {#each snap?.sources ?? [] as s (s.sourceId)}
          <div class="card status" class:failed={s.error}>
            <Icon name={s.sourceId === "reminders" ? "list" : "github"} />
            <div class="grow">
              <div>{s.label}</div>
              {#if s.error}<div class="err selectable">{s.error}</div>{/if}
            </div>
            <span class="faint">{s.syncing ? "syncing…" : relativeTime(s.lastSynced)}</span>
          </div>
        {/each}
        <button class="btn" onclick={() => store.sync()} disabled={store.syncing}><Icon name="sync" size={14} /> Sync now</button>
      {/if}
    </section>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.25);
    animation: fade 0.12s ease-out;
  }
  @media (prefers-color-scheme: dark) {
    .backdrop {
      background: rgba(0, 0, 0, 0.45);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .sheet {
    width: min(860px, calc(100vw - 48px));
    height: min(600px, calc(100vh - 48px));
    display: flex;
    border-radius: 12px;
    overflow: hidden;
    background: var(--bg);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }
  nav {
    width: 200px;
    flex: none;
    padding: 16px 10px;
    background: var(--bg-sidebar);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  nav h2 {
    font-size: 13px;
    margin: 0 8px 10px;
  }
  nav button {
    display: flex;
    align-items: center;
    gap: 9px;
    height: 28px;
    padding: 0 8px;
    border-radius: 6px;
    text-align: left;
  }
  nav button:hover {
    background: var(--bg-hover);
  }
  nav button.active {
    background: var(--bg-active);
    font-weight: 500;
  }
  .badge {
    margin-left: auto;
    padding: 0 6px;
    border-radius: 8px;
    background: var(--danger);
    color: white;
    font-size: 11px;
  }
  section {
    flex: 1;
    overflow-y: auto;
    padding: 20px 24px 28px;
    position: relative;
  }
  .close {
    position: absolute;
    top: 12px;
    right: 12px;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 17px;
  }
  h4 {
    margin: 22px 0 4px;
    font-size: 13px;
  }
  p {
    margin: 0 0 12px;
    line-height: 1.5;
  }
  code {
    font-family: var(--mono);
    font-size: 11.5px;
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--bg-active);
  }
  .card {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    margin-bottom: 6px;
    border-radius: 8px;
    border: 1px solid var(--border);
  }
  .card.off {
    opacity: 0.55;
  }
  .card.failed {
    border-color: color-mix(in srgb, var(--danger) 45%, transparent);
  }
  .account img {
    width: 28px;
    height: 28px;
    border-radius: 50%;
  }
  .warn {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 11.5px;
    color: var(--warn);
  }
  .err {
    font-size: 11.5px;
    color: var(--danger);
    white-space: pre-wrap;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .link {
    color: var(--accent);
    cursor: pointer;
  }
  .report {
    margin: 10px 0 0;
    padding-left: 18px;
    font-size: 12px;
    line-height: 1.6;
  }
  .cats {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .cat {
    display: grid;
    grid-template-columns: 32px 1fr 1fr 26px;
    gap: 8px;
    align-items: center;
  }
  .cat.head {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-3);
  }
  .cat input[type="color"] {
    width: 28px;
    height: 24px;
    padding: 0;
    border: none;
    background: none;
  }
  .icon-btn.armed {
    background: var(--danger);
    color: white;
  }
  .mono {
    font-family: var(--mono);
    font-size: 12px;
  }
</style>
