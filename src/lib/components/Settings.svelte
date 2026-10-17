<script lang="ts">
  import { api, errorText, type Category, type Provider } from "$lib/api";
  import { store, relativeTime } from "$lib/store.svelte";
  import CategorySelect from "./CategorySelect.svelte";
  import Icon from "./Icon.svelte";
  import SourcePicker from "./SourcePicker.svelte";

  const sections = [
    { id: "accounts", name: "Accounts", icon: "github" },
    { id: "repos", name: "Repositories", icon: "issue" },
    { id: "projects", name: "Projects", icon: "stack" },
    { id: "reminders", name: "Apple Reminders", icon: "list" },
    { id: "calendars", name: "Calendars", icon: "calendar" },
    { id: "categories", name: "Areas", icon: "tag" },
    { id: "display", name: "Display", icon: "eye" },
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
  let importReport = $state<Partial<Record<Provider, string[]>>>({});
  const tokenUrl =
    "https://github.com/settings/tokens/new?scopes=repo,project&description=Organizer%20app";

  async function link() {
    linking = true;
    try {
      const a = await api.addAccount(token, host, "github");
      token = "";
      store.toast(`Linked ${a.id}`, "info");
    } catch (e) {
      store.toast(errorText(e));
    } finally {
      linking = false;
    }
  }

  // ---- GitLab
  let glToken = $state("");
  let glHost = $state("gitlab.com");
  const glTokenUrl = $derived(
    `https://${glHost.trim() || "gitlab.com"}/-/user_settings/personal_access_tokens?name=Organizer&scopes=api`,
  );
  async function linkGitLab() {
    linking = true;
    try {
      const a = await api.addAccount(glToken, glHost, "gitlab");
      glToken = "";
      store.toast(`Linked ${a.id}`, "info");
    } catch (e) {
      store.toast(errorText(e));
    } finally {
      linking = false;
    }
  }

  async function runImport(provider: Provider, load: () => Promise<string[]>) {
    linking = true;
    importReport = { ...importReport, [provider]: [] };
    try {
      importReport = { ...importReport, [provider]: await load() };
    } catch (e) {
      importReport = { ...importReport, [provider]: [errorText(e)] };
    } finally {
      linking = false;
    }
  }
  const importGh = () => runImport("github", api.importGhAccounts);
  const importGlab = () => runImport("gitlab", api.importGlabAccounts);

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
  const calendarPrivacyUrl = "x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars";
  const internetAccountsUrl = "x-apple.systempreferences:com.apple.Internet-Accounts-Settings.extension";

  let requestingCalendars = $state(false);
  async function requestCalendarAccess() {
    requestingCalendars = true;
    try {
      const status = await api.requestCalendarAccess();
      if (status !== "granted") store.toast("Calendar access wasn't granted.");
    } catch (e) {
      store.toast(errorText(e));
    } finally {
      requestingCalendars = false;
    }
  }

  /** Calendars grouped by account (iCloud, Google, Exchange…). */
  const calendarGroups = $derived.by(() => {
    const groups = new Map<string, NonNullable<typeof snap>["eventCalendars"]>();
    for (const c of snap?.eventCalendars ?? []) {
      const key = c.sourceTitle ?? "Other";
      groups.set(key, [...(groups.get(key) ?? []), c]);
    }
    return [...groups.entries()];
  });

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
        <h3>Accounts</h3>
        <p class="muted">
          Link as many GitHub and GitLab accounts as you like — personal, work, school, GitHub Enterprise or a
          self-managed GitLab. Imported accounts re-read their token from <code>gh</code> or <code>glab</code> each
          time Organizer starts; a token you paste is stored in your macOS Keychain.
        </p>
        {#each snap?.accounts ?? [] as a (a.id)}
          <div class="card account">
            {#if a.avatarUrl}<img src={a.avatarUrl} alt="" />{:else}<Icon name={a.provider} size={22} />{/if}
            <div class="grow">
              <div>
                <strong>{a.name ?? a.login}</strong> <span class="faint">{a.login}@{a.host}</span>
                <span class="faint" title={a.tokenSource === "cli"
                  ? `Token read from the ${a.provider === "gitlab" ? "glab" : "gh"} CLI at launch — nothing is stored in your Keychain.`
                  : "Token stored in your macOS Keychain."}>
                  · {a.tokenSource === "cli" ? (a.provider === "gitlab" ? "via glab" : "via gh") : "Keychain"}
                </span>
              </div>
              {#if a.provider === "github" && a.scopes.length && !a.scopes.some((s) => s.includes("project"))}
                <div class="warn"><Icon name="alert" size={12} /> Missing the <code>project</code> scope — Projects won't load.</div>
              {/if}
            </div>
            <button class="btn ghost danger" onclick={() => unlink(a.id)}>{armed === `acct:${a.id}` ? "Click to confirm" : "Unlink"}</button>
          </div>
        {/each}

        <h4><Icon name="github" size={14} /> GitHub — import from the GitHub CLI</h4>
        <p class="muted">Uses every account you're logged into with <code>gh auth login</code>.</p>
        <button class="btn" onclick={importGh} disabled={linking}><Icon name="github" size={14} /> Import gh accounts</button>
        {#if importReport.github?.length}
          <ul class="report selectable">{#each importReport.github as line}<li>{line}</li>{/each}</ul>
        {/if}

        <h4>GitHub — or paste a personal access token</h4>
        <p class="muted">
          Needs <code>repo</code> and <code>project</code> scopes.
          <button class="link" onclick={() => api.openUrl(tokenUrl)}>Create one on GitHub →</button>
        </p>
        <div class="row">
          <input class="field" style="width: 150px" bind:value={host} placeholder="github.com" title="Host (change for GitHub Enterprise)" />
          <input class="field grow" type="password" bind:value={token} placeholder="ghp_… or github_pat_…" onkeydown={(e) => e.key === "Enter" && link()} />
          <button class="btn primary" onclick={link} disabled={!token.trim() || linking}>{linking ? "Linking…" : "Link"}</button>
        </div>

        <h4><Icon name="gitlab" size={14} /> GitLab — import from the GitLab CLI</h4>
        <p class="muted">Uses every host you're logged into with <code>glab auth login</code>.</p>
        <button class="btn" onclick={importGlab} disabled={linking}><Icon name="gitlab" size={14} /> Import glab accounts</button>
        {#if importReport.gitlab?.length}
          <ul class="report selectable">{#each importReport.gitlab as line}<li>{line}</li>{/each}</ul>
        {/if}

        <h4>GitLab — or paste a personal access token</h4>
        <p class="muted">
          Needs the <code>api</code> scope. Works with gitlab.com or your own GitLab server.
          <button class="link" onclick={() => api.openUrl(glTokenUrl)}>Create one on {glHost.trim() || "gitlab.com"} →</button>
        </p>
        <div class="row">
          <input class="field" style="width: 150px" bind:value={glHost} placeholder="gitlab.com" title="Host (change for a self-managed GitLab)" />
          <input class="field grow" type="password" bind:value={glToken} placeholder="glpat-…" onkeydown={(e) => e.key === "Enter" && linkGitLab()} />
          <button class="btn primary" onclick={linkGitLab} disabled={!glToken.trim() || linking}>{linking ? "Linking…" : "Link"}</button>
        </div>
      {:else if store.settingsSection === "repos"}
        <h3>Repositories</h3>
        <p class="muted">
          Open issues from these GitHub repos and GitLab projects appear in your lists. Issues labeled with an area's label (like
          <code>school</code>) go to that area; others use the repo's default area.
        </p>
        {#each snap?.repos ?? [] as r (r.id)}
          <div class="card">
            <Icon name={r.provider} />
            <div class="grow">{r.owner}/{r.name} <span class="faint">via {r.accountId}</span></div>
            <CategorySelect value={r.defaultCategory} onchange={(v) => setDefault(r.id, v)} />
            <button class="icon-btn" class:armed={armed === `src:${r.id}`} title={armed === `src:${r.id}` ? "Click again to stop tracking" : "Stop tracking (nothing changes on the server)"} onclick={() => removeSource(r.id)}><Icon name="trash" /></button>
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
      {:else if store.settingsSection === "calendars"}
        <h3>Calendars</h3>
        {#if snap?.calendarAccess === "granted"}
          <p class="muted">
            Events from every calendar on this Mac show in the Calendar views. Give each calendar a default area; a
            <code>#hashtag</code> in an event's title or notes overrides it.
          </p>
          {#each calendarGroups as [account, cals] (account)}
            <h4>{account}</h4>
            {#each cals as c (c.id)}
              <div class="card" class:off={!c.enabled}>
                <input type="checkbox" checked={c.enabled} title="Show this calendar" onchange={(e) => api.setListEnabled(c.id, e.currentTarget.checked)} />
                <span class="dot" style:background={c.color ?? "var(--text-3)"}></span>
                <div class="grow">
                  {c.title}
                  {#if !c.writable}<span class="faint">· read-only</span>{/if}
                </div>
                <CategorySelect value={c.defaultCategory} onchange={(v) => setDefault(c.id, v)} />
              </div>
            {/each}
          {:else}
            <p class="faint">No calendars found.</p>
          {/each}
        {:else if snap?.calendarAccess === "denied" || snap?.calendarAccess === "restricted" || snap?.calendarAccess === "writeOnly"}
          <p class="muted">Organizer doesn't have full access to your calendars. Turn it on in System Settings → Privacy & Security → Calendars, then come back.</p>
          <button class="btn" onclick={() => api.openUrl(calendarPrivacyUrl)}>Open Privacy Settings</button>
        {:else if snap?.calendarAccess === "unsupported"}
          <p class="muted">Calendars are only available on macOS.</p>
        {:else}
          <p class="muted">
            Organizer reads and writes events through macOS, so iCloud calendars work right away. macOS will ask you to
            allow access once.
          </p>
          <button class="btn primary" onclick={requestCalendarAccess} disabled={requestingCalendars}>
            {requestingCalendars ? "Waiting for permission…" : "Connect calendars"}
          </button>
        {/if}

        <h4>Google, Gmail and Outlook</h4>
        <p class="muted">
          Add your Google or Microsoft (Outlook, Microsoft 365, Exchange) account in System Settings → Internet Accounts
          and turn on Calendars. Its calendars then appear here — and in Apple Calendar — automatically.
        </p>
        <button class="btn" onclick={() => api.openUrl(internetAccountsUrl)}><Icon name="external" size={14} /> Open Internet Accounts</button>
      {:else if store.settingsSection === "categories"}
        <h3>Areas</h3>
        <p class="muted">
          Each area is a tab. An item lands in an area when it carries the area's <strong>label</strong> — a GitHub or GitLab label,
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
      {:else if store.settingsSection === "display"}
        <h3>Display</h3>
        <h4>Reminder notes in the list</h4>
        <div class="choices">
          {#each [["selected", "Only for the selected reminder", "Click a reminder to see its notes; click again to hide them."], ["always", "Under every reminder", "Show the first two lines of notes for every reminder."]] as [value, title, desc] (value)}
            <label class="choice" class:on={store.showNotes === value}>
              <input
                type="radio"
                name="show-notes"
                checked={store.showNotes === value}
                onchange={() => {
                  store.showNotes = value as typeof store.showNotes;
                  store.savePrefs();
                }}
              />
              <span>
                <strong>{title}</strong>
                <span class="muted">{desc}</span>
              </span>
            </label>
          {/each}
        </div>
        <h4>Calendar weeks start on</h4>
        <div class="seg-choice">
          {#each [[0, "Sunday"], [1, "Monday"]] as [value, name] (value)}
            <label class="choice inline" class:on={store.weekStart === value}>
              <input
                type="radio"
                name="week-start"
                checked={store.weekStart === value}
                onchange={() => {
                  store.weekStart = value as number;
                  store.savePrefs();
                }}
              />
              <strong>{name}</strong>
            </label>
          {/each}
        </div>
        <p class="faint small-note">
          The selected item always shows its notes in full. Previews leave out area hashtags like <code>#school</code>,
          since the area already shows on the row. GitHub descriptions only open for the selected item.
        </p>
      {:else if store.settingsSection === "status"}
        <h3>Sync status</h3>
        <p class="muted">GitHub and GitLab refresh every 2 minutes and whenever you come back to the window. Reminders and calendars update live.</p>
        {#each snap?.sources ?? [] as s (s.sourceId)}
          <div class="card status" class:failed={s.error}>
            <Icon name={s.sourceId === "reminders" ? "list" : s.sourceId === "calendar" ? "calendar" : s.sourceId.startsWith("gl:") ? "gitlab" : "github"} />
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
    display: flex;
    align-items: center;
    gap: 6px;
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
  .choices {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .choice {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid var(--border);
  }
  .choice.on {
    border-color: color-mix(in srgb, var(--accent) 60%, transparent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }
  .choice input {
    margin-top: 2px;
  }
  .choice span {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .seg-choice {
    display: flex;
    gap: 8px;
  }
  .choice.inline {
    align-items: center;
    padding: 8px 14px;
  }
  .small-note {
    margin-top: 12px;
    font-size: 12px;
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
