//! Shared data types. Everything here crosses the IPC boundary as camelCase JSON.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    /// Hex color, e.g. `#4f8cff`.
    pub color: String,
    /// GitHub label / Reminders hashtag that places an item in this category.
    pub label: String,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// `login@host`, stable across re-linking.
    pub id: String,
    pub login: String,
    pub host: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    /// GraphQL node id.
    pub id: String,
    pub account_id: String,
    pub owner: String,
    pub name: String,
    pub url: String,
    pub default_category: Option<String>,
}

impl Repo {
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// GraphQL node id.
    pub id: String,
    pub account_id: String,
    pub owner: String,
    pub number: i64,
    pub title: String,
    pub url: String,
    pub default_category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderList {
    /// EventKit `calendarIdentifier`.
    pub id: String,
    pub title: String,
    pub color: Option<String>,
    pub source_title: Option<String>,
    pub writable: bool,
    pub enabled: bool,
    pub default_category: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Reminder,
    Issue,
    PullRequest,
    DraftIssue,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Label {
    pub name: String,
    pub color: String,
}

/// A unified to-do: an Apple reminder, a GitHub issue/PR, or a project draft.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// `rem:<calendarItemIdentifier>` or `gh:<node id>`.
    pub id: String,
    pub kind: ItemKind,
    /// Reminders list id, repo node id, or project node id.
    pub container_id: String,
    pub container_name: String,
    pub account_id: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub url: Option<String>,
    pub number: Option<i64>,
    pub completed: bool,
    /// Local date (`2026-10-12`) or local date-time (`2026-10-12T14:30`).
    pub due: Option<String>,
    /// 0 = none, 1 = high, 5 = medium, 9 = low (EventKit convention).
    pub priority: i64,
    pub labels: Vec<Label>,
    pub assignees: Vec<String>,
    /// Project "Status" field value, when the item came from a project.
    pub status: Option<String>,
    /// Projects (node ids) that contain this item.
    pub project_ids: Vec<String>,
    pub updated_at: Option<String>,
    /// GitHub sub-issues: the parent issue's item id (`gh:<node id>`).
    #[serde(default)]
    pub parent_id: Option<String>,
    /// GitHub sub-issues: progress of this issue's own sub-issues.
    #[serde(default)]
    pub sub_issues: Option<SubIssues>,
    /// Resolved at read time from labels, hashtags and source defaults.
    #[serde(default)]
    pub categories: Vec<String>,
}

impl Item {
    /// For GitHub items: `owner/repo` and issue number, needed by the REST API.
    pub fn repo_and_number(&self) -> Option<(&str, &str, i64)> {
        let (owner, name) = self.container_name.split_once('/')?;
        Some((owner, name, self.number?))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SubIssues {
    pub total: i64,
    pub completed: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub source_id: String,
    pub label: String,
    pub last_synced: Option<String>,
    pub error: Option<String>,
    pub syncing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub categories: Vec<Category>,
    pub accounts: Vec<Account>,
    pub repos: Vec<Repo>,
    pub projects: Vec<Project>,
    pub reminder_lists: Vec<ReminderList>,
    pub reminders_access: String,
    pub items: Vec<Item>,
    pub sources: Vec<SourceStatus>,
}

/// What the user wants to create from the quick-add bar.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", tag = "target")]
pub enum NewItem {
    #[serde(rename_all = "camelCase")]
    Reminder {
        list_id: String,
        title: String,
        notes: Option<String>,
        due: Option<String>,
        #[serde(default)]
        priority: i64,
        category: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Issue {
        repo_id: String,
        title: String,
        body: Option<String>,
        category: Option<String>,
        #[serde(default)]
        extra_labels: Vec<String>,
        project_id: Option<String>,
        #[serde(default)]
        assign_self: bool,
    },
}

/// A repo or project the user could start tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub number: Option<i64>,
    pub url: String,
    pub description: Option<String>,
}

/// Edits from the item editor. Absent fields are left alone; an empty string
/// clears `body` or `due`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPatch {
    pub title: Option<String>,
    pub body: Option<String>,
    /// Reminders only.
    pub due: Option<String>,
    /// Reminders only.
    pub priority: Option<i64>,
    /// Reminders only: move to another list.
    pub list_id: Option<String>,
    /// GitHub issues/PRs only: the complete new label set.
    pub labels: Option<Vec<String>>,
    /// GitHub issues/PRs only: the complete new assignee set.
    pub assignees: Option<Vec<String>>,
}

/// Everything the editor needs: the item with its full (untruncated) body,
/// plus choices for labels and assignees on GitHub items.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetails {
    pub item: Item,
    pub label_options: Vec<Label>,
    pub assignee_options: Vec<String>,
}
