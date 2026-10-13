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
use crate::github::GitHub;
use crate::model::*;
use crate::reminders::{NewReminder, Reminders};
use crate::secrets::{self, Secrets};

const REMINDERS_SOURCE: &str = "reminders";
const GITHUB_INTERVAL: Duration = Duration::from_secs(120);
const REMINDERS_INTERVAL: Duration = Duration::from_secs(300);
const GITHUB_CONCURRENCY: usize = 6;

/// Tells the UI to re-fetch the snapshot.
pub const EVENT_CHANGED: &str = "state-changed";

pub struct App {
    pub db: Db,
    gh: GitHub,
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

        let rules = self.rules(&categories, &repos, &projects, &lists);
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
        futures::join!(self.sync_reminders(), self.sync_github());
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
            let token = self.secrets.get(&account.id)?;
            let items = match &source {
                SourceRef::Repo(r) => self.gh.repo_issues(&account.host, &token, r).await?,
                SourceRef::Project(p) => self.gh.project_items(&account.host, &token, p).await?,
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
            let enabled = self.db.reminder_lists()?.into_iter().filter(|l| l.enabled).map(|l| l.id).collect();
            let items = self.reminders.incomplete(enabled).await?;
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
                _ = rem_tick.tick() => self.sync_reminders().await,
                Some(()) = reminder_changes.recv() => {
                    // EventKit fires bursts of notifications; coalesce them.
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    while reminder_changes.try_recv().is_ok() {}
                    self.sync_reminders().await;
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
        let t = self.secrets.get(&a.id)?;
        Ok((a, t))
    }

    pub async fn add_account(&self, token: &str, host: &str) -> Result<Account> {
        let token = token.trim();
        let host = normalize_host(host);
        if token.is_empty() {
            return Err(Error::msg("paste a token first"));
        }
        let account = self.gh.viewer(&host, token).await?;
        self.secrets.set(&account.id, token)?;
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
                Ok(t) => match self.add_account(&t, &host).await {
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

    pub fn remove_account(&self, id: &str) -> Result<()> {
        self.db.delete_account(id)?;
        self.secrets.delete(id)?;
        self.changed();
        Ok(())
    }

    // ---- repos & projects -------------------------------------------------

    pub async fn repo_candidates(&self, account_id: &str) -> Result<Vec<Candidate>> {
        let (a, t) = self.account_token(account_id)?;
        self.gh.viewer_repos(&a.host, &t).await
    }

    pub async fn project_candidates(&self, account_id: &str) -> Result<Vec<Candidate>> {
        let (a, t) = self.account_token(account_id)?;
        self.gh.viewer_projects(&a.host, &t).await
    }

    /// Resolves `owner/name` (or a GitHub URL) to a repo.
    pub async fn lookup_repo(&self, account_id: &str, spec: &str) -> Result<Candidate> {
        let (a, t) = self.account_token(account_id)?;
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
        self.sync_reminders().await;
        Ok(())
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

    pub async fn set_completed(&self, id: &str, completed: bool) -> Result<()> {
        let (item, _) = self.item(id)?;
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
                    return Err(Error::msg("archived drafts can be restored from the project on GitHub"));
                }
            }
            ItemKind::PullRequest => {
                return Err(Error::msg("pull requests are merged or closed on GitHub"));
            }
        }
        self.update_cached(id, |i| i.completed = completed)
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
                base.completed |= other.completed;
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
        container_id: String::new(),
        container_name: String::new(),
        account_id: None,
        title: String::new(),
        body: None,
        url: None,
        number: None,
        completed: false,
        due: None,
        priority: 0,
        labels: vec![],
        assignees: vec![],
        status: None,
        project_ids: vec![],
        updated_at: None,
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
    fn slugs_and_hosts() {
        assert_eq!(slug("  Side Projects! "), "side-projects");
        assert_eq!(normalize_host(""), "github.com");
        assert_eq!(normalize_host("https://ghe.corp.com/"), "ghe.corp.com");
    }
}
