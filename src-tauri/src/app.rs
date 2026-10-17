//! Application core: builds snapshots for the UI, runs background sync, and
//! routes create/complete/categorize actions to Apple Reminders or GitHub.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::UnboundedReceiver;

use crate::categorize::{Rules, hashtags};
use crate::db::Db;
use crate::error::{Error, Result};
use crate::github::{self, GitHub};
use crate::gitlab::{self, GitLab};
use crate::model::*;
use crate::reminders::{NewReminder, ReminderPatch, Reminders};
use crate::secrets::{self, Secrets};

const REMINDERS_SOURCE: &str = "reminders";
const CALENDAR_SOURCE: &str = "calendar";
/// Events are fetched for this window around today.
const EVENTS_PAST_DAYS: i64 = 90;
const EVENTS_FUTURE_DAYS: i64 = 400;
const GITHUB_INTERVAL: Duration = Duration::from_secs(120);
/// How far back the Completed view reaches. Items finished before this drop out
/// of the cache on the next sync of their source.
const COMPLETED_DAYS: i64 = 30;
const REMINDERS_INTERVAL: Duration = Duration::from_secs(300);
const GITHUB_CONCURRENCY: usize = 6;

/// The oldest completion time the app keeps.
fn completed_cutoff() -> chrono::DateTime<chrono::Local> {
    chrono::Local::now() - chrono::Duration::days(COMPLETED_DAYS)
}

/// Tells the UI to re-fetch the snapshot.
pub const EVENT_CHANGED: &str = "state-changed";

pub struct App {
    pub db: Db,
    gh: GitHub,
    gl: GitLab,
    secrets: Secrets,
    reminders: Reminders,
    handle: AppHandle,
    syncing: Mutex<HashSet<String>>,
}

/// Marks a source as syncing for as long as it's alive.
struct SyncGuard<'a> {
    app: &'a App,
    id: String,
}

impl Drop for SyncGuard<'_> {
    fn drop(&mut self) {
        self.app.syncing.lock().unwrap().remove(&self.id);
        self.app.changed();
    }
}

impl App {
    pub fn new(handle: AppHandle, db: Db, reminders: Reminders) -> Self {
        Self {
            db,
            gh: GitHub::new(),
            gl: GitLab::new(),
            secrets: Secrets::default(),
            reminders,
            handle,
            syncing: Mutex::new(HashSet::new()),
        }
    }

    fn changed(&self) {
        let _ = self.handle.emit(EVENT_CHANGED, ());
    }

    // ---- snapshot ---------------------------------------------------------

    pub fn snapshot(&self) -> Result<Snapshot> {
        let categories = self.db.categories()?;
        let accounts = self.db.accounts()?;
        let repos = self.db.repos()?;
        let projects = self.db.projects()?;
        let lists = self.db.reminder_lists()?;
        let event_calendars = self.db.event_calendars()?;

        let all_lists: Vec<ReminderList> = lists.iter().chain(&event_calendars).cloned().collect();
        let rules = self.rules(&categories, &repos, &projects, &all_lists);
        let enabled_lists: HashSet<&str> =
            lists.iter().filter(|l| l.enabled).map(|l| l.id.as_str()).collect();

        let items = merge_copies(self.db.all_items()?)
            .into_iter()
            .filter(|i| i.kind != ItemKind::Reminder || enabled_lists.contains(i.container_id.as_str()))
            .map(|mut i| {
                i.categories = rules.categorize(&i);
                i
            })
            .collect();

        let enabled_calendars: HashSet<&str> =
            event_calendars.iter().filter(|c| c.enabled).map(|c| c.id.as_str()).collect();
        let events = self
            .db
            .events()?
            .into_iter()
            .filter(|e| enabled_calendars.contains(e.calendar_id.as_str()))
            .map(|mut e| {
                e.categories = rules.categorize_event(&e);
                e
            })
            .collect();

        let syncing = self.syncing.lock().unwrap().clone();
        let mut sources = Vec::new();
        let mut status = |id: &str, label: String| -> Result<()> {
            let (last_synced, error) = self.db.sync_state(id)?;
            sources.push(SourceStatus {
                source_id: id.into(),
                label,
                last_synced,
                error,
                syncing: syncing.contains(id),
            });
            Ok(())
        };
        status(REMINDERS_SOURCE, "Apple Reminders".into())?;
        if self.reminders.calendar_access_status() == "granted" {
            status(CALENDAR_SOURCE, "Calendars".into())?;
        }
        for r in &repos {
            status(&r.id, r.full_name())?;
        }
        for p in &projects {
            status(&p.id, p.title.clone())?;
        }

        Ok(Snapshot {
            categories,
            accounts,
            repos,
            projects,
            reminder_lists: lists,
            reminders_access: self.reminders.access_status().into(),
            event_calendars,
            calendar_access: self.reminders.calendar_access_status().into(),
            events,
            items,
            sources,
        })
    }

    fn rules(&self, categories: &[Category], repos: &[Repo], projects: &[Project], lists: &[ReminderList]) -> Rules {
        let defaults = repos
            .iter()
            .map(|r| (r.id.as_str(), r.default_category.as_deref()))
            .chain(projects.iter().map(|p| (p.id.as_str(), p.default_category.as_deref())))
            .chain(lists.iter().map(|l| (l.id.as_str(), l.default_category.as_deref())));
        Rules::new(categories, defaults)
    }

    // ---- sync -------------------------------------------------------------

    fn begin_sync(&self, id: &str) -> Option<SyncGuard<'_>> {
        if !self.syncing.lock().unwrap().insert(id.to_string()) {
            return None; // already running
        }
        self.changed();
        Some(SyncGuard { app: self, id: id.into() })
    }

    pub async fn sync_all(&self) {
        futures::join!(self.sync_reminders(), self.sync_events(), self.sync_github());
    }

    pub async fn sync_events(&self) {
        if self.reminders.calendar_access_status() != "granted" {
            return;
        }
        let Some(_guard) = self.begin_sync(CALENDAR_SOURCE) else { return };
        let res = async {
            let cals = self.reminders.event_calendars().await?;
            self.db.sync_event_calendars(&cals)?;
            let enabled = self.db.event_calendars()?.into_iter().filter(|c| c.enabled).map(|c| c.id).collect();
            let today = chrono::Local::now().date_naive();
            let events = self
                .reminders
                .events(
                    enabled,
                    today - chrono::Duration::days(EVENTS_PAST_DAYS),
                    today + chrono::Duration::days(EVENTS_FUTURE_DAYS),
                )
                .await?;
            self.db.replace_events(&events)
        }
        .await;
        if let Err(e) = res {
            tracing::warn!("calendar sync failed: {e}");
            let _ = self.db.set_sync_error(CALENDAR_SOURCE, &e.to_string());
        }
    }

    pub async fn sync_github(&self) {
        let repos = self.db.repos().unwrap_or_default();
        let projects = self.db.projects().unwrap_or_default();
        let jobs = repos
            .into_iter()
            .map(SourceRef::Repo)
            .chain(projects.into_iter().map(SourceRef::Project));
        futures::stream::iter(jobs)
            .for_each_concurrent(GITHUB_CONCURRENCY, |s| async move { self.sync_source(s).await })
            .await;
    }

    async fn sync_source(&self, source: SourceRef) {
        let id = source.id().to_string();
        let Some(_guard) = self.begin_sync(&id) else { return };
        let res = async {
            let account = self.account(source.account_id())?;
            let token = self.secrets.token(&account)?;
            let since = completed_cutoff().to_rfc3339();
            let items = match &source {
                SourceRef::Repo(r) if r.provider == Provider::Gitlab => {
                    self.gl.issues(&account.host, &token, r, &since).await?
                }
                SourceRef::Repo(r) => self.gh.repo_issues(&account.host, &token, r, &since).await?,
                SourceRef::Project(p) => self.gh.project_items(&account.host, &token, p, &since).await?,
            };
            self.db.replace_items(&id, &items)
        }
        .await;
        if let Err(e) = res {
            tracing::warn!("sync {id} failed: {e}");
            let _ = self.db.set_sync_error(&id, &e.to_string());
        }
    }

    pub async fn sync_reminders(&self) {
        if self.reminders.access_status() != "granted" {
            return;
        }
        let Some(_guard) = self.begin_sync(REMINDERS_SOURCE) else { return };
        let res = async {
            let lists = self.reminders.lists().await?;
            self.db.sync_reminder_lists(&lists)?;
            let enabled: Vec<String> =
                self.db.reminder_lists()?.into_iter().filter(|l| l.enabled).map(|l| l.id).collect();
            let mut items = self.reminders.incomplete(enabled.clone()).await?;
            // Recently completed reminders, for the Completed view.
            items.extend(self.reminders.completed(enabled, completed_cutoff().naive_local()).await?);
            self.db.replace_items(REMINDERS_SOURCE, &items)
        }
        .await;
        if let Err(e) = res {
            tracing::warn!("reminders sync failed: {e}");
            let _ = self.db.set_sync_error(REMINDERS_SOURCE, &e.to_string());
        }
    }

    /// Periodic sync, plus near-instant Reminders refresh on EventKit change notifications.
    pub async fn run_background(self: Arc<Self>, mut reminder_changes: UnboundedReceiver<()>) {
        self.sync_all().await;
        let mut gh_tick = tokio::time::interval(GITHUB_INTERVAL);
        let mut rem_tick = tokio::time::interval(REMINDERS_INTERVAL);
        gh_tick.tick().await;
        rem_tick.tick().await;
        loop {
            tokio::select! {
                _ = gh_tick.tick() => {
                    let app = self.clone();
                    tokio::spawn(async move { app.sync_github().await });
                }
                _ = rem_tick.tick() => {
                    futures::join!(self.sync_reminders(), self.sync_events());
                }
                Some(()) = reminder_changes.recv() => {
                    // EventKit fires bursts of notifications (for reminders and
                    // events alike); coalesce them.
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    while reminder_changes.try_recv().is_ok() {}
                    futures::join!(self.sync_reminders(), self.sync_events());
                }
            }
        }
    }

    // ---- accounts ---------------------------------------------------------

    fn account(&self, id: &str) -> Result<Account> {
        self.db.account(id)?.ok_or_else(|| Error::msg("account not found"))
    }

    fn account_token(&self, id: &str) -> Result<(Account, String)> {
        let a = self.account(id)?;
        let t = self.secrets.token(&a)?;
        Ok((a, t))
    }

    /// Links an account from a token the user pasted, which goes in the Keychain.
    pub async fn add_account(&self, token: &str, host: &str, provider: Provider) -> Result<Account> {
        self.link_account(token, host, provider, TokenSource::Keychain).await
    }

    async fn link_account(
        &self,
        token: &str,
        host: &str,
        provider: Provider,
        source: TokenSource,
    ) -> Result<Account> {
        let token = token.trim();
        if token.is_empty() {
            return Err(Error::msg("paste a token first"));
        }
        let mut account = match provider {
            Provider::Github => self.gh.viewer(&normalize_host(host), token).await?,
            Provider::Gitlab => self.gl.viewer(&normalize_gitlab_host(host), token).await?,
        };
        account.token_source = source;
        match source {
            TokenSource::Keychain => self.secrets.set(&account.id, token)?,
            // `gh`/`glab` hold the credential; keep it for this run only, and
            // drop anything an earlier link left in the Keychain. That delete
            // can fail when the item was written by another build, which is
            // the one thing we don't want to block the import on.
            TokenSource::Cli => {
                let _ = self.secrets.delete(&account.id);
                self.secrets.remember(&account.id, token);
            }
        }
        self.db.upsert_account(&account)?;
        self.changed();
        Ok(account)
    }

    /// Imports every account the GitHub CLI is logged into. Returns one line per account.
    pub async fn import_gh_accounts(&self) -> Result<Vec<String>> {
        let found = tokio::task::spawn_blocking(secrets::gh_cli_accounts)
            .await
            .map_err(|e| Error::msg(e.to_string()))?;
        if found.is_empty() {
            return Err(Error::msg(
                "No GitHub CLI accounts found. Install `gh` and run `gh auth login`, or paste a token instead.",
            ));
        }
        let mut report = Vec::new();
        for (host, user) in found {
            let (h, u) = (host.clone(), user.clone());
            let token = tokio::task::spawn_blocking(move || secrets::gh_cli_token(&h, &u))
                .await
                .map_err(|e| Error::msg(e.to_string()))?;
            let line = match token {
                Ok(t) => match self.link_account(&t, &host, Provider::Github, TokenSource::Cli).await {
                    Ok(a) if a.scopes.is_empty() || a.scopes.iter().any(|s| s.contains("project")) => {
                        format!("✓ {}", a.id)
                    }
                    Ok(a) => format!(
                        "✓ {} — no `project` scope, so Projects won't load. Run `gh auth refresh -s project` and import again.",
                        a.id
                    ),
                    Err(e) => format!("✗ {user}@{host}: {e}"),
                },
                Err(e) => format!("✗ {user}@{host}: {e}"),
            };
            report.push(line);
        }
        Ok(report)
    }

    /// Imports every GitLab host the GitLab CLI is logged into. One line per host.
    pub async fn import_glab_accounts(&self) -> Result<Vec<String>> {
        let found = tokio::task::spawn_blocking(secrets::glab_cli_accounts)
            .await
            .map_err(|e| Error::msg(e.to_string()))?;
        if found.is_empty() {
            return Err(Error::msg(
                "No GitLab CLI accounts found. Install `glab` and run `glab auth login`, or paste a token instead.",
            ));
        }
        let mut report = Vec::new();
        for host in found {
            let h = host.clone();
            let token = tokio::task::spawn_blocking(move || secrets::glab_cli_token(&h))
                .await
                .map_err(|e| Error::msg(e.to_string()))?;
            let line = match token {
                Ok(t) => match self.link_account(&t, &host, Provider::Gitlab, TokenSource::Cli).await {
                    Ok(a) if a.scopes.is_empty() || a.scopes.iter().any(|s| s == "api") => format!("✓ {}", a.id),
                    Ok(a) => format!(
                        "✓ {} — no `api` scope, so issues won't load. Run `glab auth login --hostname {host}` with an `api` token.",
                        a.id
                    ),
                    Err(e) => format!("✗ {host}: {e}"),
                },
                Err(e) => format!("✗ {host}: {e}"),
            };
            report.push(line);
        }
        Ok(report)
    }

    pub fn remove_account(&self, id: &str) -> Result<()> {
        self.db.delete_account(id)?;
        self.secrets.delete(id)?;
        self.changed();
        Ok(())
    }

    // ---- repos & projects -------------------------------------------------

    pub async fn repo_candidates(&self, account_id: &str) -> Result<Vec<Candidate>> {
        let (a, t) = self.account_token(account_id)?;
        match a.provider {
            Provider::Github => self.gh.viewer_repos(&a.host, &t).await,
            Provider::Gitlab => self.gl.projects(&a.host, &t).await,
        }
    }

    pub async fn project_candidates(&self, account_id: &str) -> Result<Vec<Candidate>> {
        let (a, t) = self.account_token(account_id)?;
        if a.provider == Provider::Gitlab {
            return Err(Error::msg("Projects are a GitHub feature — pick a GitHub account"));
        }
        self.gh.viewer_projects(&a.host, &t).await
    }

    /// Resolves `owner/name` (or a GitHub URL) to a repo.
    pub async fn lookup_repo(&self, account_id: &str, spec: &str) -> Result<Candidate> {
        let (a, t) = self.account_token(account_id)?;
        if a.provider == Provider::Gitlab {
            let path = parse_gitlab_path(spec).ok_or_else(|| Error::msg("use the form group/project or paste the project URL"))?;
            return self.gl.project(&a.host, &t, &path).await;
        }
        let (owner, name) = parse_repo_spec(spec).ok_or_else(|| Error::msg("use the form owner/repo"))?;
        self.gh.repo(&a.host, &t, &owner, &name).await
    }

    /// Resolves `owner/number` or a project URL to a project.
    pub async fn lookup_project(&self, account_id: &str, spec: &str) -> Result<Candidate> {
        let (a, t) = self.account_token(account_id)?;
        let (owner, number) =
            parse_project_spec(spec).ok_or_else(|| Error::msg("use the form owner/number or paste the project URL"))?;
        self.gh.project_by_number(&a.host, &t, &owner, number).await
    }

    pub async fn add_repo(&self, account_id: &str, c: Candidate, category: Option<String>) -> Result<()> {
        let repo = Repo {
            id: c.id,
            account_id: account_id.into(),
            owner: c.owner,
            name: c.name,
            url: c.url,
            default_category: category,
            provider: self.account(account_id)?.provider,
        };
        self.db.upsert_repo(&repo)?;
        self.changed();
        self.sync_source(SourceRef::Repo(repo)).await;
        Ok(())
    }

    pub async fn add_project(&self, account_id: &str, c: Candidate, category: Option<String>) -> Result<()> {
        let project = Project {
            id: c.id,
            account_id: account_id.into(),
            owner: c.owner,
            number: c.number.unwrap_or_default(),
            title: c.name,
            url: c.url,
            default_category: category,
        };
        self.db.upsert_project(&project)?;
        self.changed();
        self.sync_source(SourceRef::Project(project)).await;
        Ok(())
    }

    pub fn remove_source(&self, id: &str) -> Result<()> {
        self.db.delete_source(id)?;
        self.changed();
        Ok(())
    }

    pub fn set_default_category(&self, container_id: &str, category: Option<&str>) -> Result<()> {
        self.db.set_default_category(container_id, category)?;
        self.changed();
        Ok(())
    }

    // ---- reminders setup --------------------------------------------------

    pub async fn request_reminders_access(&self) -> Result<String> {
        self.reminders.request_access().await?;
        self.sync_reminders().await;
        Ok(self.reminders.access_status().into())
    }

    pub async fn set_list_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.db.set_list_enabled(id, enabled)?;
        self.changed();
        futures::join!(self.sync_reminders(), self.sync_events());
        Ok(())
    }

    pub async fn request_calendar_access(&self) -> Result<String> {
        self.reminders.request_calendar_access().await?;
        self.sync_events().await;
        Ok(self.reminders.calendar_access_status().into())
    }

    // ---- events -----------------------------------------------------------

    fn event(&self, id: &str, fallback: Option<CalendarEvent>) -> Result<CalendarEvent> {
        match self.db.event(id)? {
            Some(e) => Ok(e),
            None => fallback
                .filter(|f| f.id == id)
                .ok_or_else(|| Error::msg("event not found — try refreshing")),
        }
    }

    pub async fn create_event(&self, new: NewEvent) -> Result<CalendarEvent> {
        let cal = self
            .db
            .event_calendars()?
            .into_iter()
            .find(|c| c.id == new.calendar_id)
            .ok_or_else(|| Error::msg("pick a calendar"))?;
        // Like reminders: tag the event if its calendar doesn't imply the area.
        let mut notes = new.notes.filter(|n| !n.trim().is_empty());
        if let Some(cat) = new.category.filter(|c| cal.default_category.as_ref() != Some(c)) {
            let tag = format!("#{}", self.category(&cat)?.label);
            notes = Some(match notes {
                Some(n) => format!("{n}\n{tag}"),
                None => tag,
            });
        }
        let event = self
            .reminders
            .create_event(new.calendar_id, new.title, new.start, new.end, new.all_day, new.location, notes)
            .await?;
        self.db.put_event(&event)?;
        self.changed();
        Ok(event)
    }

    pub async fn update_event(
        &self,
        id: &str,
        patch: EventPatch,
        fallback: Option<CalendarEvent>,
    ) -> Result<CalendarEvent> {
        let event = self.event(id, fallback)?;
        if patch.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            return Err(Error::msg("the title can't be empty"));
        }
        let updated = self.reminders.update_event(event.event_id.clone(), event.occurrence, patch).await?;
        // Ids are stable across moves (see `CalendarEvent::id`), but replace
        // defensively in case EventKit detached the occurrence.
        self.db.remove_event(id)?;
        self.db.put_event(&updated)?;
        self.changed();
        Ok(updated)
    }

    /// Deletes an event. Only used to undo creating one.
    pub async fn delete_event(&self, id: &str, fallback: Option<CalendarEvent>) -> Result<()> {
        let event = self.event(id, fallback)?;
        self.reminders.remove_event(event.event_id, event.occurrence).await?;
        self.db.remove_event(id)?;
        self.changed();
        Ok(())
    }

    pub fn event_url(&self, id: &str) -> Result<String> {
        let event = self.event(id, None)?;
        Ok(format!("ical://ekevent/{}?method=show&options=more", event.event_id))
    }

    // ---- categories -------------------------------------------------------

    pub fn upsert_category(&self, mut c: Category) -> Result<Category> {
        c.name = c.name.trim().to_string();
        if c.name.is_empty() {
            return Err(Error::msg("category needs a name"));
        }
        if c.label.trim().is_empty() {
            c.label = slug(&c.name);
        }
        if c.id.is_empty() {
            c.id = slug(&c.name);
            let existing = self.db.categories()?;
            if existing.iter().any(|e| e.id == c.id) {
                return Err(Error::msg(format!("a category called \"{}\" already exists", c.name)));
            }
            c.position = existing.iter().map(|e| e.position + 1).max().unwrap_or(0);
        }
        self.db.upsert_category(&c)?;
        self.changed();
        Ok(c)
    }

    pub fn delete_category(&self, id: &str) -> Result<()> {
        self.db.delete_category(id)?;
        self.changed();
        Ok(())
    }

    // ---- items ------------------------------------------------------------

    fn category(&self, id: &str) -> Result<Category> {
        self.db
            .categories()?
            .into_iter()
            .find(|c| c.id == id)
            .ok_or_else(|| Error::msg("unknown category"))
    }

    fn item(&self, id: &str) -> Result<(Item, Vec<String>)> {
        let copies = self.db.item_copies(id)?;
        let sources = copies.iter().map(|(s, _)| s.clone()).collect();
        let item = merge_copies(copies).pop().ok_or_else(|| Error::msg("item not found — try refreshing"))?;
        Ok((item, sources))
    }

    /// Like `item`, but falls back to a copy supplied by the UI. Undo needs this:
    /// by the time you undo a completion, a sync may have dropped the item.
    fn item_or(&self, id: &str, fallback: Option<Item>) -> Result<Item> {
        match self.item(id) {
            Ok((item, _)) => Ok(item),
            Err(e) => fallback.filter(|f| f.id == id).ok_or(e),
        }
    }

    /// Re-caches an item that no source currently holds, so it shows again
    /// right away instead of after the next sync.
    fn ensure_cached(&self, item: &Item) -> Result<()> {
        if !self.db.item_copies(&item.id)?.is_empty() {
            return Ok(());
        }
        let source = if item.kind == ItemKind::Reminder {
            REMINDERS_SOURCE.to_string()
        } else if self.db.repos()?.iter().any(|r| r.id == item.container_id) {
            item.container_id.clone()
        } else {
            item.project_ids.first().cloned().unwrap_or_else(|| item.container_id.clone())
        };
        self.db.put_item(&source, item)
    }

    /// Applies `f` to every cached copy of an item.
    fn update_cached(&self, id: &str, f: impl Fn(&mut Item)) -> Result<()> {
        for (src, mut it) in self.db.item_copies(id)? {
            f(&mut it);
            self.db.put_item(&src, &it)?;
        }
        self.changed();
        Ok(())
    }

    pub async fn create_item(&self, new: NewItem) -> Result<Item> {
        let item = match new {
            NewItem::Reminder { list_id, title, notes, due, priority, category } => {
                let list = self
                    .db
                    .reminder_lists()?
                    .into_iter()
                    .find(|l| l.id == list_id)
                    .ok_or_else(|| Error::msg("pick a Reminders list"))?;
                // If the list doesn't already imply the category, tag the reminder.
                let mut notes = notes.filter(|n| !n.trim().is_empty());
                if let Some(cat) = category.filter(|c| list.default_category.as_ref() != Some(c)) {
                    let tag = format!("#{}", self.category(&cat)?.label);
                    notes = Some(match notes {
                        Some(n) => format!("{n}\n{tag}"),
                        None => tag,
                    });
                }
                let item = self
                    .reminders
                    .create(NewReminder { list_id, title, notes, due, priority })
                    .await?;
                self.db.put_item(REMINDERS_SOURCE, &item)?;
                item
            }
            NewItem::Issue { repo_id, title, body, category, extra_labels, project_id, assign_self } => {
                let repo = self
                    .db
                    .repos()?
                    .into_iter()
                    .find(|r| r.id == repo_id)
                    .ok_or_else(|| Error::msg("pick a repository"))?;
                let (account, token) = self.account_token(&repo.account_id)?;
                let mut labels = extra_labels;
                if let Some(cat) = category {
                    labels.push(self.category(&cat)?.label);
                }
                if repo.provider == Provider::Gitlab {
                    let item = self
                        .gl
                        .create_issue(&account.host, &token, &repo, &title, body.as_deref(), &labels, assign_self)
                        .await?;
                    self.db.put_item(&repo.id, &item)?;
                    self.changed();
                    return Ok(item);
                }
                let assignees = if assign_self { vec![account.login.clone()] } else { vec![] };
                let mut item = self
                    .gh
                    .create_issue(&account.host, &token, &repo, &title, body.as_deref(), &labels, &assignees)
                    .await?;
                if let Some(pid) = project_id {
                    let content_id = item.id.trim_start_matches("gh:").to_string();
                    self.gh.add_to_project(&account.host, &token, &pid, &content_id).await?;
                    item.project_ids = vec![pid];
                }
                self.db.put_item(&repo.id, &item)?;
                item
            }
        };
        self.changed();
        Ok(item)
    }

    pub async fn set_completed(&self, id: &str, completed: bool, fallback: Option<Item>) -> Result<()> {
        let item = self.item_or(id, fallback)?;
        if item.provider == Provider::Gitlab {
            let event = if completed { "close" } else { "reopen" };
            self.gitlab_update(&item, serde_json::json!({ "state_event": event })).await?;
            return Ok(());
        }
        match item.kind {
            ItemKind::Reminder => {
                let raw = id.trim_start_matches("rem:").to_string();
                self.reminders.set_completed(raw, completed).await?;
            }
            ItemKind::Issue => {
                let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
                let (owner, name, number) =
                    item.repo_and_number().ok_or_else(|| Error::msg("issue is missing its repository"))?;
                self.gh.set_issue_closed(&account.host, &token, owner, name, number, completed).await?;
            }
            ItemKind::DraftIssue => {
                let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
                let pitem = id.trim_start_matches("ghpi:");
                if completed {
                    self.gh.archive_project_item(&account.host, &token, &item.container_id, pitem).await?;
                } else {
                    self.gh.unarchive_project_item(&account.host, &token, &item.container_id, pitem).await?;
                }
            }
            ItemKind::PullRequest => {
                return Err(Error::msg("pull requests are merged or closed on GitHub"));
            }
        }
        if !completed {
            self.ensure_cached(&Item { completed: false, completed_at: None, ..item })?;
        }
        let now = chrono::Local::now().to_rfc3339();
        self.update_cached(id, |i| {
            i.completed = completed;
            i.completed_at = completed.then(|| now.clone());
        })
    }

    /// Changes an item's start and/or due date (the Gantt chart's drag and
    /// resize). `None` leaves a date alone; `Some("")` clears it. Reminders
    /// have no start date, so only `due` applies to them.
    pub async fn set_dates(
        &self,
        id: &str,
        start: Option<String>,
        due: Option<String>,
        fallback: Option<Item>,
    ) -> Result<Item> {
        let item = self.item_or(id, fallback)?;
        if item.provider == Provider::Gitlab {
            if start.is_some() {
                return Err(Error::msg("GitLab issues have a due date but no start date"));
            }
            return self.gitlab_patch(&item, ItemPatch { due, ..Default::default() }).await;
        }
        if item.kind == ItemKind::Reminder {
            if start.is_some() {
                return Err(Error::msg("reminders don't have start dates"));
            }
            return self.update_item(id, ItemPatch { due, ..Default::default() }, Some(item)).await;
        }
        let dates = item.project_dates.clone().ok_or_else(|| {
            Error::msg("this item's dates come from a milestone or iteration — add it to a project with date fields to edit them here")
        })?;
        let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
        let normalize = |d: Option<String>| d.map(|d| d.chars().take(10).collect::<String>());
        let (start, due) = (normalize(start), normalize(due));
        for (value, field, label) in [(&start, &dates.start_field, "start"), (&due, &dates.due_field, "due")] {
            let Some(value) = value else { continue };
            let field = field
                .as_deref()
                .ok_or_else(|| Error::msg(format!("this project has no {label} date field")))?;
            let date = Some(value.as_str()).filter(|v| !v.is_empty());
            self.gh.set_project_date(&account.host, &token, &dates, field, date).await?;
        }
        let apply = |i: &mut Item| {
            if let Some(s) = &start {
                i.start = Some(s.clone()).filter(|s| !s.is_empty());
            }
            if let Some(d) = &due {
                i.due = Some(d.clone()).filter(|d| !d.is_empty());
            }
        };
        let mut updated = item.clone();
        apply(&mut updated);
        self.ensure_cached(&updated)?;
        self.update_cached(id, apply)?;
        Ok(updated)
    }

    /// Undoes creating an item. A reminder is deleted; a GitHub issue is closed
    /// as "not planned" (deleting issues needs admin rights).
    pub async fn discard_item(&self, id: &str) -> Result<()> {
        let (item, _) = self.item(id)?;
        if item.provider == Provider::Gitlab {
            // Deleting needs owner rights, so undo closes the new issue.
            let (account, token, repo) = self.gitlab_repo(&item)?;
            self.gl
                .update_issue(&account.host, &token, &repo, item.number.unwrap_or_default(), serde_json::json!({ "state_event": "close" }))
                .await?;
            self.db.remove_item(id)?;
            self.changed();
            return Ok(());
        }
        match item.kind {
            ItemKind::Reminder => {
                self.reminders.remove(id.trim_start_matches("rem:").to_string()).await?;
                self.db.remove_item(id)?;
            }
            ItemKind::Issue => {
                let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
                let (owner, name, number) =
                    item.repo_and_number().ok_or_else(|| Error::msg("issue is missing its repository"))?;
                self.gh.close_not_planned(&account.host, &token, owner, name, number).await?;
                self.db.remove_item(id)?;
            }
            _ => return Err(Error::msg("only new reminders and issues can be undone")),
        }
        self.changed();
        Ok(())
    }

    /// Moves an item to another category tab. For GitHub this swaps category
    /// labels; for Reminders it moves the reminder to a list mapped to the
    /// category, or tags it with `#label` if no list is mapped.
    pub async fn set_category(&self, id: &str, category: Option<String>) -> Result<()> {
        let (item, _) = self.item(id)?;
        let categories = self.db.categories()?;
        let target = category.as_deref().map(|c| self.category(c)).transpose()?;
        let is_category_tag = |t: &str| {
            let t = t.to_lowercase();
            categories.iter().any(|c| c.label.to_lowercase() == t || c.name.to_lowercase() == t)
        };

        if item.provider == Provider::Gitlab {
            // GitLab takes the whole label list at once.
            let mut labels: Vec<String> =
                item.labels.iter().map(|l| l.name.clone()).filter(|l| !is_category_tag(l)).collect();
            labels.extend(target.as_ref().map(|t| t.label.clone()));
            self.gitlab_update(&item, serde_json::json!({ "labels": labels.join(",") })).await?;
            return Ok(());
        }

        match item.kind {
            ItemKind::Issue | ItemKind::PullRequest => {
                let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
                let (owner, name, number) =
                    item.repo_and_number().ok_or_else(|| Error::msg("issue is missing its repository"))?;
                for l in item.labels.iter().filter(|l| is_category_tag(&l.name)) {
                    if target.as_ref().is_some_and(|t| t.label.eq_ignore_ascii_case(&l.name)) {
                        continue;
                    }
                    self.gh.remove_label(&account.host, &token, owner, name, number, &l.name).await?;
                }
                if let Some(t) = &target {
                    self.gh.add_label(&account.host, &token, owner, name, number, &t.label).await?;
                }
                let new_label = target.as_ref().map(|t| Label { name: t.label.clone(), color: "ededed".into() });
                self.update_cached(id, |i| {
                    i.labels.retain(|l| !is_category_tag(&l.name));
                    i.labels.extend(new_label.clone());
                })
            }
            ItemKind::DraftIssue => Err(Error::msg(
                "draft issues can't have labels — convert it to an issue on GitHub, or set the project's default category",
            )),
            ItemKind::Reminder => {
                let raw = id.trim_start_matches("rem:").to_string();
                // Strip existing category hashtags so they don't override the move.
                let cleaned_notes = item.body.as_deref().map(|b| strip_tags(b, &is_category_tag));
                let mapped_list = target.as_ref().and_then(|t| {
                    self.db.reminder_lists().ok()?.into_iter().find(|l| {
                        l.enabled && l.writable && l.default_category.as_deref() == Some(t.id.as_str())
                    })
                });
                let current_default = self
                    .db
                    .reminder_lists()?
                    .into_iter()
                    .find(|l| l.id == item.container_id)
                    .and_then(|l| l.default_category);

                let notes = match (&target, &mapped_list) {
                    (Some(_), Some(_)) | (None, _) => cleaned_notes.clone(),
                    (Some(t), None) => Some(match cleaned_notes.as_deref() {
                        Some(n) if !n.is_empty() => format!("{n}\n#{}", t.label),
                        _ => format!("#{}", t.label),
                    }),
                };
                if target.is_none() && current_default.is_some() {
                    return Err(Error::msg(
                        "this reminder's list has a default category — move it to an unmapped list to send it to the Inbox",
                    ));
                }
                if notes != item.body {
                    self.reminders.set_notes(raw.clone(), notes.filter(|n| !n.is_empty())).await?;
                }
                if let Some(list) = mapped_list.filter(|l| l.id != item.container_id) {
                    self.reminders.move_to_list(raw, list.id).await?;
                }
                self.sync_reminders().await;
                Ok(())
            }
        }
    }

    // ---- editing ----------------------------------------------------------

    /// Loads an item for the editor. GitHub bodies are fetched fresh because the
    /// cache truncates them, and saving a truncated body would lose text.
    pub async fn item_details(&self, id: &str) -> Result<ItemDetails> {
        let (mut item, _) = self.item(id)?;
        if item.provider == Provider::Gitlab {
            let (account, token, repo) = self.gitlab_repo(&item)?;
            let iid = item.number.unwrap_or_default();
            let (issue, labels, members) = futures::try_join!(
                self.gl.issue(&account.host, &token, &repo, iid),
                self.gl.labels(&account.host, &token, &repo),
                self.gl.members(&account.host, &token, &repo),
            )?;
            let mut fresh = issue.item;
            fresh.body = issue.body;
            fresh.categories = item.categories;
            return Ok(ItemDetails {
                item: fresh,
                label_options: labels,
                assignee_options: members.into_iter().map(|(_, u)| u).collect(),
            });
        }
        let mut details = ItemDetails { item: item.clone(), label_options: vec![], assignee_options: vec![] };
        match item.kind {
            ItemKind::Reminder => {}
            ItemKind::Issue | ItemKind::PullRequest => {
                let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
                let (owner, name, number) =
                    item.repo_and_number().ok_or_else(|| Error::msg("issue is missing its repository"))?;
                let (issue, labels, assignees) = futures::try_join!(
                    self.gh.issue(&account.host, &token, owner, name, number),
                    self.gh.repo_labels(&account.host, &token, owner, name),
                    self.gh.repo_assignees(&account.host, &token, owner, name),
                )?;
                item.title = issue.title;
                item.body = issue.body;
                item.labels = issue.labels;
                item.assignees = issue.assignees;
                details.label_options = labels;
                details.assignee_options = assignees;
                details.item = item;
            }
            ItemKind::DraftIssue => {
                let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
                let (_, title, body) = self.gh.draft_issue(&account.host, &token, id.trim_start_matches("ghpi:")).await?;
                item.title = title;
                item.body = Some(body).filter(|b| !b.trim().is_empty());
                details.item = item;
            }
        }
        Ok(details)
    }

    pub async fn update_item(&self, id: &str, patch: ItemPatch, fallback: Option<Item>) -> Result<Item> {
        let item = self.item_or(id, fallback)?;
        if item.provider == Provider::Gitlab {
            return self.gitlab_patch(&item, patch).await;
        }
        if patch.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
            return Err(Error::msg("the title can't be empty"));
        }
        let title = patch.title.map(|t| t.trim().to_string());
        let updated = match item.kind {
            ItemKind::Reminder => {
                let raw = id.trim_start_matches("rem:").to_string();
                let clear_empty = |v: String| Some(v).filter(|v| !v.trim().is_empty());
                let updated = self
                    .reminders
                    .update(
                        raw,
                        ReminderPatch {
                            title,
                            notes: patch.body.map(clear_empty),
                            due: patch.due.map(clear_empty),
                            priority: patch.priority,
                            list_id: patch.list_id,
                        },
                    )
                    .await?;
                self.db.put_item(REMINDERS_SOURCE, &updated)?;
                updated
            }
            ItemKind::Issue | ItemKind::PullRequest => {
                let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
                let (owner, name, number) =
                    item.repo_and_number().ok_or_else(|| Error::msg("issue is missing its repository"))?;
                let mut fields = serde_json::Map::new();
                if let Some(t) = title {
                    fields.insert("title".into(), t.into());
                }
                if let Some(b) = patch.body {
                    fields.insert("body".into(), b.into());
                }
                if let Some(l) = patch.labels {
                    fields.insert("labels".into(), l.into());
                }
                if let Some(a) = patch.assignees {
                    fields.insert("assignees".into(), a.into());
                }
                if fields.is_empty() {
                    return Ok(item);
                }
                let issue = self
                    .gh
                    .update_issue(&account.host, &token, owner, name, number, fields.into())
                    .await?;
                let body = github::cached_body(issue.body.as_deref());
                let apply = |i: &mut Item| {
                    i.title = issue.title.clone();
                    i.body = body.clone();
                    i.labels = issue.labels.clone();
                    i.assignees = issue.assignees.clone();
                    i.updated_at = Some(issue.updated_at.clone());
                };
                let mut updated = item.clone();
                apply(&mut updated);
                self.ensure_cached(&updated)?;
                self.update_cached(id, apply)?;
                updated
            }
            ItemKind::DraftIssue => {
                let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
                let (draft_id, old_title, old_body) =
                    self.gh.draft_issue(&account.host, &token, id.trim_start_matches("ghpi:")).await?;
                let title = title.unwrap_or(old_title);
                let body = patch.body.unwrap_or(old_body);
                self.gh.update_draft(&account.host, &token, &draft_id, &title, &body).await?;
                let cached = github::cached_body(Some(&body));
                let apply = |i: &mut Item| {
                    i.title = title.clone();
                    i.body = cached.clone();
                };
                let mut updated = item.clone();
                apply(&mut updated);
                self.ensure_cached(&updated)?;
                self.update_cached(id, apply)?;
                updated
            }
        };
        self.changed();
        Ok(updated)
    }

    pub fn item_url(&self, id: &str) -> Result<String> {
        let (item, _) = self.item(id)?;
        match item.kind {
            ItemKind::Reminder => {
                Ok(format!("x-apple-reminderkit://REMCDReminder/{}", id.trim_start_matches("rem:")))
            }
            _ => item.url.ok_or_else(|| Error::msg("no link for this item")),
        }
    }
}

// ---- GitLab -------------------------------------------------------------------

impl App {
    /// The account, token and (tracked or reconstructed) repo behind a GitLab issue.
    fn gitlab_repo(&self, item: &Item) -> Result<(Account, String, Repo)> {
        let (account, token) = self.account_token(item.account_id.as_deref().unwrap_or_default())?;
        let repo = match self.db.repos()?.into_iter().find(|r| r.id == item.container_id) {
            Some(r) => r,
            None => {
                let (owner, name) = item.container_name.rsplit_once('/').unwrap_or(("", &item.container_name));
                Repo {
                    id: item.container_id.clone(),
                    account_id: account.id.clone(),
                    owner: owner.into(),
                    name: name.into(),
                    url: String::new(),
                    default_category: None,
                    provider: Provider::Gitlab,
                }
            }
        };
        Ok((account, token, repo))
    }

    /// Sends a GitLab issue update and caches the result.
    async fn gitlab_update(&self, item: &Item, fields: serde_json::Value) -> Result<Item> {
        let (account, token, repo) = self.gitlab_repo(item)?;
        let iid = item.number.ok_or_else(|| Error::msg("issue is missing its number"))?;
        let mut updated = self.gl.update_issue(&account.host, &token, &repo, iid, fields).await?.item;
        updated.categories = vec![];
        // Keep showing it for undo even once closed; the next sync drops it.
        self.db.put_item(&repo.id, &updated)?;
        self.changed();
        Ok(updated)
    }

    /// Applies editor changes to a GitLab issue.
    async fn gitlab_patch(&self, item: &Item, patch: ItemPatch) -> Result<Item> {
        let mut fields = serde_json::Map::new();
        if let Some(t) = patch.title {
            fields.insert("title".into(), t.trim().into());
        }
        if let Some(b) = patch.body {
            fields.insert("description".into(), b.into());
        }
        if let Some(l) = patch.labels {
            fields.insert("labels".into(), l.join(",").into());
        }
        if let Some(d) = patch.due {
            let d: String = d.chars().take(10).collect();
            fields.insert("due_date".into(), if d.is_empty() { serde_json::Value::Null } else { d.into() });
        }
        if let Some(a) = patch.assignees {
            let (account, token, repo) = self.gitlab_repo(item)?;
            let members = self.gl.members(&account.host, &token, &repo).await?;
            fields.insert("assignee_ids".into(), gitlab::assignee_ids(&members, &a)?.into());
        }
        if fields.is_empty() {
            return Ok(item.clone());
        }
        self.gitlab_update(item, fields.into()).await
    }
}

enum SourceRef {
    Repo(Repo),
    Project(Project),
}

impl SourceRef {
    fn id(&self) -> &str {
        match self {
            SourceRef::Repo(r) => &r.id,
            SourceRef::Project(p) => &p.id,
        }
    }
    fn account_id(&self) -> &str {
        match self {
            SourceRef::Repo(r) => &r.account_id,
            SourceRef::Project(p) => &p.account_id,
        }
    }
}

/// Collapses copies of the same item cached by different sources (an issue
/// tracked via its repo and via a project). The most recently updated copy
/// wins; project membership and status are unioned. Order is stable.
fn merge_copies(rows: Vec<(String, Item)>) -> Vec<Item> {
    let mut index: HashMap<String, usize> = HashMap::with_capacity(rows.len());
    let mut out: Vec<Item> = Vec::with_capacity(rows.len());
    for (_, item) in rows {
        match index.get(&item.id) {
            None => {
                index.insert(item.id.clone(), out.len());
                out.push(item);
            }
            Some(&i) => {
                let existing = &mut out[i];
                let (mut base, other) = if item.updated_at > existing.updated_at {
                    (item, std::mem::replace(existing, placeholder()))
                } else {
                    (std::mem::replace(existing, placeholder()), item)
                };
                for p in other.project_ids {
                    if !base.project_ids.contains(&p) {
                        base.project_ids.push(p);
                    }
                }
                base.status = base.status.or(other.status);
                // The project copy's dates win: they're the ones the Gantt chart edits.
                if other.project_dates.is_some() {
                    base.start = other.start.or(base.start);
                    base.due = other.due.or(base.due);
                    base.project_dates = other.project_dates;
                } else {
                    base.start = base.start.or(other.start);
                    base.due = base.due.or(other.due);
                }
                base.completed |= other.completed;
        base.completed_at = base.completed_at.or(other.completed_at);
                *existing = base;
            }
        }
    }
    out
}

fn placeholder() -> Item {
    Item {
        id: String::new(),
        kind: ItemKind::Issue,
        provider: Provider::Github,
        container_id: String::new(),
        container_name: String::new(),
        account_id: None,
        title: String::new(),
        body: None,
        url: None,
        number: None,
        completed: false,
        completed_at: None,
        due: None,
        priority: 0,
        labels: vec![],
        assignees: vec![],
        status: None,
        project_ids: vec![],
        updated_at: None,
        start: None,
        project_dates: None,
        parent_id: None,
        sub_issues: None,
        categories: vec![],
    }
}

fn strip_tags(text: &str, is_tag: &dyn Fn(&str) -> bool) -> String {
    let drop: HashSet<String> = hashtags(text).filter(|t| is_tag(t)).collect();
    text.lines()
        .map(|line| {
            line.split(' ')
                .filter(|w| {
                    !w.strip_prefix('#')
                        .map(|t| drop.contains(&t.trim_end_matches(|c: char| c.is_ascii_punctuation()).to_lowercase()))
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn slug(s: &str) -> String {
    let s: String = s
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

fn normalize_host(host: &str) -> String {
    let h = host.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
    if h.is_empty() || h == "api.github.com" { "github.com".into() } else { h.into() }
}

fn normalize_gitlab_host(host: &str) -> String {
    let h = host.trim().trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
    if h.is_empty() { "gitlab.com".into() } else { h.into() }
}

/// `group/sub/project` from a path or a project/issue URL.
fn parse_gitlab_path(spec: &str) -> Option<String> {
    let s = spec.trim().trim_end_matches('/').trim_end_matches(".git");
    let s = s.split_once("://").map_or(s, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
    // Drop anything after GitLab's `/-/` separator (issues, tree, …).
    let path = s.split("/-/").next()?.trim_matches('/');
    (path.contains('/') && !path.is_empty()).then(|| path.to_string())
}

fn parse_repo_spec(spec: &str) -> Option<(String, String)> {
    let s = spec.trim().trim_end_matches('/').trim_end_matches(".git");
    let s = s.split_once("://").map_or(s, |(_, rest)| rest.split_once('/').map_or("", |(_, p)| p));
    let mut parts = s.split('/').filter(|p| !p.is_empty());
    let (owner, name) = (parts.next()?, parts.next()?);
    Some((owner.into(), name.into()))
}

/// Accepts `owner/3`, `owner#3`, or `https://github.com/orgs/owner/projects/3`.
fn parse_project_spec(spec: &str) -> Option<(String, i64)> {
    let s = spec.trim().trim_end_matches('/');
    if s.contains("://") {
        let parts: Vec<&str> = s.split('/').collect();
        let i = parts.iter().position(|p| *p == "projects")?;
        return Some((parts.get(i.checked_sub(1)?)?.to_string(), parts.get(i + 1)?.parse().ok()?));
    }
    let (owner, n) = s.split_once(['/', '#'])?;
    Some((owner.into(), n.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(updated: &str, project: Option<&str>, status: Option<&str>) -> Item {
        let mut i = placeholder();
        i.id = "gh:I_1".into();
        i.title = format!("v{updated}");
        i.updated_at = Some(updated.into());
        i.project_ids = project.into_iter().map(Into::into).collect();
        i.status = status.map(Into::into);
        i
    }

    #[test]
    fn merges_repo_and_project_copies() {
        let rows = vec![
            ("R1".into(), issue("2026-01-02", None, None)),
            ("P1".into(), issue("2026-01-01", Some("P1"), Some("In progress"))),
            ("P2".into(), issue("2026-01-03", Some("P2"), None)),
        ];
        let out = merge_copies(rows);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "v2026-01-03");
        assert_eq!(out[0].project_ids, vec!["P2", "P1"]);
        assert_eq!(out[0].status.as_deref(), Some("In progress"));
    }

    #[test]
    fn merge_prefers_the_project_copys_dates() {
        let mut repo = issue("2026-01-02", None, None);
        repo.due = Some("2026-03-01".into()); // milestone
        let mut proj = issue("2026-01-01", Some("P1"), None);
        proj.start = Some("2026-02-01".into());
        proj.due = Some("2026-02-10".into());
        proj.project_dates = Some(ProjectDates {
            project_id: "P1".into(),
            item_id: "PVTI".into(),
            start_field: Some("S".into()),
            due_field: Some("D".into()),
        });
        let out = merge_copies(vec![("R1".into(), repo), ("P1".into(), proj)]);
        assert_eq!(out[0].start.as_deref(), Some("2026-02-01"));
        assert_eq!(out[0].due.as_deref(), Some("2026-02-10"));
        assert!(out[0].project_dates.is_some());
    }

    #[test]
    fn merge_keeps_a_project_date_from_an_older_copy() {
        let mut from_project = issue("2026-01-01", Some("P1"), None);
        from_project.due = Some("2026-02-01".into());
        let out = merge_copies(vec![("P1".into(), from_project), ("R1".into(), issue("2026-01-02", None, None))]);
        assert_eq!(out[0].due.as_deref(), Some("2026-02-01"));
    }

    #[test]
    fn strips_only_category_tags() {
        let is_cat = |t: &str| ["school", "fun"].contains(&t);
        assert_eq!(strip_tags("call mom #School #urgent\n#fun.", &is_cat), "call mom #urgent");
    }

    #[test]
    fn parses_specs() {
        assert_eq!(parse_repo_spec("rust-lang/rust"), Some(("rust-lang".into(), "rust".into())));
        assert_eq!(
            parse_repo_spec("https://github.com/tauri-apps/tauri.git"),
            Some(("tauri-apps".into(), "tauri".into()))
        );
        assert_eq!(parse_repo_spec("nope"), None);
        assert_eq!(parse_project_spec("acme/12"), Some(("acme".into(), 12)));
        assert_eq!(
            parse_project_spec("https://github.com/orgs/acme/projects/4/views/1"),
            Some(("acme".into(), 4))
        );
        assert_eq!(parse_project_spec("https://github.com/users/me/projects/2"), Some(("me".into(), 2)));
    }

    #[test]
    fn item_patch_reads_camel_case_and_omits_missing_fields() {
        let p: ItemPatch =
            serde_json::from_str(r#"{"title":"New","due":"","listId":"L2","labels":["bug","school"]}"#).unwrap();
        assert_eq!(p.title.as_deref(), Some("New"));
        assert_eq!(p.due.as_deref(), Some(""));
        assert_eq!(p.list_id.as_deref(), Some("L2"));
        assert_eq!(p.labels.unwrap(), vec!["bug", "school"]);
        assert!(p.body.is_none() && p.priority.is_none() && p.assignees.is_none());
    }

    #[test]
    fn parses_gitlab_paths() {
        assert_eq!(parse_gitlab_path("group/sub/app").as_deref(), Some("group/sub/app"));
        assert_eq!(
            parse_gitlab_path("https://gitlab.com/group/sub/app/-/issues/12").as_deref(),
            Some("group/sub/app")
        );
        assert_eq!(parse_gitlab_path("https://git.corp.dev/team/app.git").as_deref(), Some("team/app"));
        assert_eq!(parse_gitlab_path("app"), None);
        assert_eq!(normalize_gitlab_host(""), "gitlab.com");
    }

    #[test]
    fn slugs_and_hosts() {
        assert_eq!(slug("  Side Projects! "), "side-projects");
        assert_eq!(normalize_host(""), "github.com");
        assert_eq!(normalize_host("https://ghe.corp.com/"), "ghe.corp.com");
    }
}
